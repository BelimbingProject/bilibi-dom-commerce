//! Commerce sales: catalog items, quotations and sales orders. All writes use
//! the reviewed contracts in operations; pages and agents share that door.
mod contributions;
mod delivery;
mod operations;
mod prepare;
mod print;
mod store;
mod web;

use bilibi_agent_api::OperationClient;
use bilibi_database::{Db, Error, Result};
use bilibi_tenancy::Scope;
use serde_json::Value;

pub const MODULE: &str = "commerce/sales";
pub const SERVICE: &str = "commerce/sales.sales";
pub const VIEW: &str = "commerce.sales.view";
pub const MANAGE: &str = "commerce.sales.manage";
pub const CONFIRM: &str = "commerce.sales.confirm";

/// A stored item or document. Facts contain exact decimal strings. Document
/// facts and lines are immutable; revision fences every status transition.
#[derive(Clone, Debug)]
pub struct Record {
    pub id: i64,
    pub revision: i64,
    pub kind: String,
    pub code: String,
    pub status: String,
    pub facts: Value,
}
impl Record {
    pub fn snapshot(&self) -> Value {
        serde_json::json!({"id":self.id,"revision":self.revision,"kind":self.kind,
            "code":self.code,"status":self.status,"facts":self.facts})
    }
}

/// The read facade. Write callers use the registered operation contracts,
/// which authorize and validate on the audited transaction they receive.
#[derive(Clone)]
pub struct Sales {
    db: Db,
}
impl Sales {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
    #[allow(clippy::too_many_arguments)] // Bounded query with explicit company and kind.
    pub async fn list(
        &self,
        scope: &Scope,
        company: i64,
        kind: &str,
        search: &str,
        status: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Record>> {
        let client = self.db.get().await?;
        authorize(&**client, scope, company, VIEW).await?;
        store::list(
            &**client,
            scope.tenant(),
            company,
            kind,
            search,
            status,
            limit.clamp(1, 100),
            offset.max(0),
        )
        .await
    }
    pub async fn get(&self, scope: &Scope, company: i64, kind: &str, id: i64) -> Result<Record> {
        let client = self.db.get().await?;
        authorize(&**client, scope, company, VIEW).await?;
        let mut record = store::get(&**client, scope.tenant(), company, kind, id)
            .await?
            .ok_or(Error::NotFound)?;
        if kind == "order" {
            delivery::progress(&**client, scope, company, &mut record).await?;
        }
        Ok(record)
    }
    pub async fn history(&self, scope: &Scope, company: i64, id: i64) -> Result<Vec<Value>> {
        let client = self.db.get().await?;
        authorize(&**client, scope, company, VIEW).await?;
        store::history(&**client, scope.tenant(), company, id).await
    }
}
async fn authorize(
    client: &dyn OperationClient,
    scope: &Scope,
    company: i64,
    key: &str,
) -> Result<()> {
    bilibi_company::authorize_work_target(client, scope, company, key)
        .await
        .map(|_| ())
}
