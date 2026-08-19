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
use crate::read;
use crate::verb::Verb;

/// One caller's session over the store.
pub struct Session {
    client: Client,
    actor: String,
    graph: Option<String>,
    /// Serializes whole calls. The client pipelines concurrent queries
    /// happily, and that is exactly wrong here: a read verb sharing the
    /// connection could run between SET ROLE and RESET ROLE. One call at
    /// a time is the session's contract, held by lock rather than by
    /// hope.
    serial: tokio::sync::Mutex<()>,
    /// Set when an escalation begins, cleared only when RESET ROLE
    /// completes. A cancelled or failed reset leaves it set, and a set
    /// flag retires the session: refusing every further call is the
    /// session-level form of discarding a connection that may still be
    /// escalated.
    escalated: std::sync::atomic::AtomicBool,
}

impl Session {
    /// Open a session for an actor the caller has already established.
    ///
    /// Not reachable over the wire: the actor arrives here from the
    /// kernel by way of the daemon, or from a test that names itself.
    pub fn new(client: Client, actor: impl Into<String>) -> Self {
        Self {
            client,
            actor: actor.into(),
            graph: None,
            serial: tokio::sync::Mutex::new(()),
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

    /// The connection, for the verb modules that emit SQL.
    pub(crate) fn client(&self) -> &Client {
        &self.client
    }

    /// The session's graph, if it has one.
    pub(crate) fn graph(&self) -> Option<&str> {
        self.graph.as_deref()
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

    /// Run one verb: record the attempt, dispatch, mark the outcome, and
    /// answer in the envelope either way.
    pub async fn call(&self, verb: &Verb) -> Envelope {
        let name = verb.wire_name();
        // One call at a time on this connection (finding: pipelined
        // queries would otherwise interleave with an escalation).
        let _serial = self.serial.lock().await;
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
        let attempt = match audit::begin(self.client(), &self.actor, name, &args).await {
            Ok(a) => a,
            Err(e) => return error_envelope(name, &e),
        };

        let result = self.dispatch(verb).await;
        audit::finish(self.client(), attempt, result.as_ref().map(|_| ())).await;

        match result {
            Ok(data) => envelope(name, data),
            Err(e) => error_envelope(name, &e),
        }
    }

    /// The exhaustive match. Verbs of later phases name the phase they
    /// wait for rather than panicking or pretending (EC-1).
    async fn dispatch(&self, verb: &Verb) -> Result<Value, VerbError> {
        match verb {
            // -- Phase 2, this spec ---------------------------------------
            Verb::Orient(r) => read::orient(self, r).await,
            Verb::Status(_) => read::status(self).await,
            Verb::Health(_) => read::health(self).await,
            Verb::Check(r) => read::check(self, r).await,
            Verb::Stats(r) => read::stats(self, r).await,
            Verb::CodebaseStats(r) => read::codebase_stats(self, r).await,
            Verb::Get(r) => read::get(self, r).await,
            Verb::List(r) => read::list(self, r).await,
            Verb::Count(r) => read::count(self, r).await,
            Verb::Recent(r) => read::recent(self, r).await,
            Verb::Query(r) => read::query(self, r).await,
            Verb::SchemaVersion(_) => read::schema_version(),

            // -- Phase 3, spec 013 -----------------------------------------
            Verb::GraphTraverse(r) => graph::traverse(self, r).await,
            Verb::GraphNeighbors(r) => graph::neighbors(self, r).await,
            Verb::GraphShortestPath(r) => graph::shortest_path(self, r).await,
            Verb::GraphList(_) => graph::list(self).await,
            Verb::GraphCreate(r) => graph::create(self, r).await,
            Verb::GraphDrop(r) => graph::drop(self, r).await,
            Verb::DatabaseList(_) => database::list(self).await,
            Verb::DatabaseCreate(r) => database::create(self, r).await,
            Verb::DatabaseDrop(r) => database::drop(self, r).await,
            // R14: the name is bound by R4, the meaning is not yet
            // anyone's. It refuses until a consumer defines it.
            Verb::GraphMaterialize(_) => Err(VerbError::Unimplemented(
                "graph.materialize waits for a consumer that defines materialization (R14)".into(),
            )),

            Verb::Insert(_) | Verb::Update(_) | Verb::Delete(_) | Verb::Purge(_) | Verb::Sql(_) => {
                Err(unimplemented_in("Phase 4", verb))
            }

            Verb::SchemaApply(_) | Verb::SchemaList(_) | Verb::SchemaShow(_) => {
                Err(unimplemented_in("the schema manager, H7", verb))
            }

            Verb::EmbedText(_) => Err(unimplemented_in("the embedder, H4", verb)),

            Verb::GraphEmbedEmbed(_) | Verb::GraphEmbedNeighbors(_) | Verb::GraphEmbedUpdate(_) => {
                Err(unimplemented_in("the graph-embed era, H9", verb))
            }

            Verb::Ingest(_)
            | Verb::CodebaseIngest(_)
            | Verb::CodebaseRetire(_)
            | Verb::CodebasePrune(_)
            | Verb::CodebaseDrift(_)
            | Verb::CodebaseValidate(_) => Err(unimplemented_in("Phase 6", verb)),
        }
    }
}

/// A verb whose contract exists and whose implementation does not, saying
/// which piece of work it waits for.
fn unimplemented_in(what: &str, verb: &Verb) -> VerbError {
    VerbError::Unimplemented(format!("{} arrives with {what}", verb.wire_name()))
}
