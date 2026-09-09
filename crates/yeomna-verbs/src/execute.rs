//! The session and the dispatch: where a verb stops being a shape and
//! becomes a call.
//!
//! One session, one connection, one actor. The daemon (Phase 5) will
//! construct one per client and fill the actor from `SO_PEERCRED`. Nothing
//! here reads the actor from a request, because the contract has no such
//! field and V3 is the reason.
//!
//! Dispatch is an exhaustive match over the closed vocabulary. A verb
//! added later fails to compile until it is handled or refused by name,
//! which is what G2's closed enum buys.

use serde_json::Value;
use tokio_postgres::Client;

use crate::audit;
use crate::database;
use crate::envelope::{Envelope, envelope, error_envelope};
use crate::error::VerbError;
use crate::graph;
use crate::ingest;
use crate::read;
use crate::sql;
use crate::verb::Verb;
use crate::write;

/// One caller's session over the store.
pub struct Session {
    /// R16 (spec 014): the client lives inside the lock that serializes
    /// whole calls. The lock used to guard a unit and the client sat
    /// beside it, until `Client::transaction` demanded `&mut Client` and
    /// the answer was to merge the two: holding the lock is what yields
    /// the exclusive borrow, so one mechanism owns both invariants
    /// (finding: pipelined queries would otherwise interleave with an
    /// escalation).
    client: tokio::sync::Mutex<Client>,
    actor: String,
    graph: Option<String>,
    /// Where this cluster answers, for the one verb that opens a second
    /// connection (`sql` reaches its target database directly). A session
    /// built without it refuses that verb rather than guessing.
    endpoint: Option<(String, u16)>,
    /// Set when an escalation begins, cleared only when RESET ROLE
    /// completes. A cancelled or failed reset leaves it set, and a set
    /// flag retires the session: refusing every further call is the
    /// session-level form of discarding a connection that may still be
    /// escalated.
    escalated: std::sync::atomic::AtomicBool,
}

/// One call's view of the session, lent to the verb modules.
///
/// Exists because the client lives inside the call lock (R16): dispatch
/// holds the guard and lends the borrow here, so a verb implementation
/// can never reach the connection without the serialization that makes
/// the reach safe.
pub(crate) struct Exec<'a> {
    client: &'a Client,
    actor: &'a str,
    graph: Option<&'a str>,
    escalated: &'a std::sync::atomic::AtomicBool,
}

impl Exec<'_> {
    /// The connection, for the verb modules that emit SQL.
    pub(crate) fn client(&self) -> &Client {
        self.client
    }

    /// The session's graph, if it has one.
    pub(crate) fn graph(&self) -> Option<&str> {
        self.graph
    }

    /// Who this session calls as. Reported by `status` so a caller can
    /// ask the appliance who it thinks the caller is, which is the only
    /// way to see the answer without reading the audit log, and the
    /// audit log has no verb.
    pub(crate) fn actor(&self) -> &str {
        self.actor
    }

    /// Run one statement as `yeomna_provision`.
    ///
    /// The escalation flag is set before SET ROLE and cleared only after
    /// RESET ROLE completes, so a failure or a cancellation mid-flight
    /// leaves it set and the session retires itself (spec 013 EC-5 as
    /// amended by review): a connection that cannot prove it was reset
    /// is never used again. Cancellation-safe because the flag is raised
    /// eagerly rather than lowered in a destructor.
    pub(crate) async fn as_provision(&self, sql: &str) -> Result<(), tokio_postgres::Error> {
        use std::sync::atomic::Ordering;
        self.escalated.store(true, Ordering::SeqCst);
        if let Err(e) = self.client.batch_execute("SET ROLE yeomna_provision").await {
            // An error from SET ROLE means the role never changed, so
            // the session is not escalated and must not retire over it:
            // a missing provision role would otherwise brick every
            // session that tried. A cancellation during the await never
            // reaches this line, which is the case the eager flag exists
            // for, since the statement may have taken effect server-side
            // with nobody left polling.
            self.escalated.store(false, Ordering::SeqCst);
            return Err(e);
        }
        let result = self.client.batch_execute(sql).await;
        let reset = self.client.batch_execute("RESET ROLE").await;
        if reset.is_ok() {
            self.escalated.store(false, Ordering::SeqCst);
        }
        result?;
        reset
    }
}

impl Session {
    /// Open a session for an actor the caller has already established.
    ///
    /// Not reachable over the wire: the actor arrives here from the
    /// kernel by way of the daemon, or from a test that names itself.
    pub fn new(client: Client, actor: impl Into<String>) -> Self {
        Self {
            client: tokio::sync::Mutex::new(client),
            actor: actor.into(),
            graph: None,
            endpoint: None,
            escalated: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Scope this session's document reads to one graph. Absent, they
    /// span the database, which `get` treats as an ambiguity to report
    /// rather than a choice to make (EC-2).
    pub fn with_graph(mut self, graph: impl Into<String>) -> Self {
        self.graph = Some(graph.into());
        self
    }

    /// Tell the session where its cluster answers, so the `sql` verb can
    /// open its per-call connection to a target database (R17). The
    /// values are the ones the session's own client was built from.
    pub fn with_endpoint(mut self, socket_dir: impl Into<String>, port: u16) -> Self {
        self.endpoint = Some((socket_dir.into(), port));
        self
    }

    /// Where this cluster answers, for the verbs that open their own
    /// connection: `sql` reaches another database, and the ingesting
    /// verbs need a client the call lock is not holding.
    fn endpoint(&self) -> Option<(&str, u16)> {
        self.endpoint
            .as_ref()
            .map(|(dir, port)| (dir.as_str(), *port))
    }

    fn exec<'a>(&'a self, client: &'a Client) -> Exec<'a> {
        Exec {
            client,
            actor: &self.actor,
            graph: self.graph.as_deref(),
            escalated: &self.escalated,
        }
    }

    /// Run one verb: record the attempt, dispatch, mark the outcome, and
    /// answer in the envelope either way.
    pub async fn call(&self, verb: &Verb) -> Envelope {
        let name = verb.wire_name();
        // One call at a time on this connection, and the lock is also
        // where the exclusive client borrow for transactions comes from
        // (R16).
        let mut client = self.client.lock().await;
        // A session whose last escalation never proved its reset is
        // retired, not reused.
        if self.escalated.load(std::sync::atomic::Ordering::SeqCst) {
            return error_envelope(
                name,
                &VerbError::Internal(
                    "this session is retired: a role escalation was not confirmed reset".into(),
                ),
            );
        }
        let args = serde_json::to_value(verb)
            .ok()
            .and_then(|v| v.get("args").cloned())
            .unwrap_or(Value::Null);

        // EC-5: an attempt that cannot be recorded is not made.
        let attempt = match audit::begin(&client, &self.actor, name, &args).await {
            Ok(a) => a,
            Err(e) => return error_envelope(name, &e),
        };

        let result = self.dispatch(&mut client, attempt, verb).await;
        // A write verb that committed marked its own outcome inside the
        // transaction (FR5: an ok on a write is durable if and only if
        // the mutation is), and `finish` guards on a NULL outcome, so
        // this mark is terminal everywhere else and a no-op there.
        audit::finish(&client, attempt, result.as_ref().map(|_| ())).await;

        match result {
            Ok(data) => envelope(name, data),
            Err(e) => error_envelope(name, &e),
        }
    }

    /// The exhaustive match. Verbs of later phases name the phase they
    /// wait for rather than panicking or pretending (EC-1).
    async fn dispatch(
        &self,
        client: &mut Client,
        attempt: audit::Attempt,
        verb: &Verb,
    ) -> Result<Value, VerbError> {
        let graph = self.graph.as_deref();
        match verb {
            // -- Phase 2, spec 012 -----------------------------------------
            Verb::Orient(r) => read::orient(&self.exec(client), r).await,
            Verb::Status(_) => read::status(&self.exec(client)).await,
            Verb::Health(_) => read::health(&self.exec(client)).await,
            Verb::Check(r) => read::check(&self.exec(client), r).await,
            Verb::Stats(r) => read::stats(&self.exec(client), r).await,
            Verb::CodebaseStats(r) => read::codebase_stats(&self.exec(client), r).await,
            Verb::Get(r) => read::get(&self.exec(client), r).await,
            Verb::List(r) => read::list(&self.exec(client), r).await,
            Verb::Count(r) => read::count(&self.exec(client), r).await,
            Verb::Recent(r) => read::recent(&self.exec(client), r).await,
            Verb::Query(r) => read::query(&self.exec(client), r).await,
            Verb::SchemaVersion(_) => read::schema_version(),

            // -- Phase 3, spec 013 -----------------------------------------
            Verb::GraphTraverse(r) => graph::traverse(&self.exec(client), r).await,
            Verb::GraphNeighbors(r) => graph::neighbors(&self.exec(client), r).await,
            Verb::GraphShortestPath(r) => graph::shortest_path(&self.exec(client), r).await,
            Verb::GraphList(_) => graph::list(&self.exec(client)).await,
            Verb::GraphCreate(r) => graph::create(&self.exec(client), r).await,
            Verb::GraphDrop(r) => graph::drop(&self.exec(client), r).await,
            Verb::DatabaseList(_) => database::list(&self.exec(client)).await,
            Verb::DatabaseCreate(r) => database::create(&self.exec(client), r).await,
            Verb::DatabaseDrop(r) => database::drop(&self.exec(client), r).await,
            // R14: the name is bound by R4, the meaning is not yet
            // anyone's. It refuses until a consumer defines it.
            Verb::GraphMaterialize(_) => Err(VerbError::Unimplemented(
                "graph.materialize waits for a consumer that defines materialization (R14)".into(),
            )),

            // -- Phase 4, spec 014 -----------------------------------------
            Verb::Insert(r) => write::insert(client, graph, attempt, r).await,
            Verb::Update(r) => write::update(client, graph, attempt, r).await,
            Verb::Delete(r) => write::delete(client, graph, attempt, r).await,
            Verb::Purge(r) => write::purge(client, graph, attempt, r).await,
            Verb::EdgeAssert(r) => write::edge_assert(client, graph, attempt, r).await,
            Verb::EdgeRetract(r) => write::edge_retract(client, graph, attempt, r).await,
            Verb::Sql(r) => sql::sql(&self.exec(client), self.endpoint(), r).await,

            Verb::SchemaApply(_) | Verb::SchemaList(_) | Verb::SchemaShow(_) => {
                Err(unimplemented_in("the schema manager, H7", verb))
            }

            Verb::EmbedText(_) => Err(unimplemented_in("the embedder, H4", verb)),

            Verb::GraphEmbedEmbed(_) | Verb::GraphEmbedNeighbors(_) | Verb::GraphEmbedUpdate(_) => {
                Err(unimplemented_in("the graph-embed era, H9", verb))
            }

            // -- Phase 6a, spec 019 ----------------------------------------
            Verb::Ingest(r) => ingest::ingest(&self.exec(client), self.endpoint(), r).await,
            Verb::CodebaseIngest(r) => {
                ingest::codebase_ingest(&self.exec(client), self.endpoint(), r).await
            }
            Verb::CodebaseDrift(r) => {
                ingest::codebase_drift(&self.exec(client), self.endpoint(), r).await
            }
            Verb::CodebaseValidate(r) => ingest::codebase_validate(&self.exec(client), r).await,

            // The destructive half, whose audit args carry the sentence
            // T3's truth rests on.
            Verb::CodebaseRetire(_) | Verb::CodebasePrune(_) => {
                Err(unimplemented_in("Phase 6b", verb))
            }
        }
    }
}

/// A verb whose contract exists and whose implementation does not, saying
/// which piece of work it waits for.
fn unimplemented_in(what: &str, verb: &Verb) -> VerbError {
    VerbError::Unimplemented(format!("{} arrives with {what}", verb.wire_name()))
}
