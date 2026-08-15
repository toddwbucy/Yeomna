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
use crate::envelope::{Envelope, envelope, error_envelope};
use crate::error::VerbError;
use crate::read;
use crate::verb::Verb;

/// One caller's session over the store.
pub struct Session {
    client: Client,
    actor: String,
    graph: Option<String>,
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

    /// Run one verb: record the attempt, dispatch, mark the outcome, and
    /// answer in the envelope either way.
    pub async fn call(&self, verb: &Verb) -> Envelope {
        let name = verb.wire_name();
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

            // -- Later phases, named ---------------------------------------
            Verb::GraphTraverse(_)
            | Verb::GraphNeighbors(_)
            | Verb::GraphShortestPath(_)
            | Verb::GraphList(_)
            | Verb::GraphCreate(_)
            | Verb::GraphDrop(_)
            | Verb::GraphMaterialize(_)
            | Verb::DatabaseList(_)
            | Verb::DatabaseCreate(_)
            | Verb::DatabaseDrop(_) => Err(unimplemented_in("Phase 3", verb)),

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
