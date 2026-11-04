use crate::{prepare, store, CONFIRM, MANAGE, MODULE};
use bilibi_agent_api::{
    Channels, Definitions, Error, Handler, Operation, OperationClient, OperationFuture,
    WriteHandler, DEFINITIONS,
};
use bilibi_database::MutationTransaction;
use bilibi_events::{Events, NewEvent};
use bilibi_module_registry::{Contribution, Provenance};
use bilibi_records::Records;
use bilibi_tenancy::{PartyProofs, Scope};
use serde_json::{json, Value};
use std::sync::{Arc, OnceLock};

pub(crate) const CONFIRMED: &str = "commerce/sales.order_confirmed";
pub(crate) const STATUS_CHANGED: &str = "commerce/sales.order_status_changed";
struct Services {
    events: Arc<Events>,
    records: Arc<Records>,
    parties: Arc<PartyProofs>,
}
static SERVICES: OnceLock<Services> = OnceLock::new();
pub(crate) fn bind(
    events: Arc<Events>,
    records: Arc<Records>,
    parties: Arc<PartyProofs>,
) -> bilibi_module_registry::Result<()> {
    SERVICES
        .set(Services {
            events,
            records,
            parties,
        })
        .map_err(|_| {
            bilibi_module_registry::RegistryError::Service("duplicate commerce binding".into())
        })
}
struct Write(&'static str);
impl Write {
    async fn prepare(
        &self,
        c: &dyn OperationClient,
        s: &Scope,
        v: &Value,
    ) -> bilibi_agent_api::Result<prepare::Prepared> {
        let company = prepare::integer(v, "company_id")?;
        crate::authorize(c, s, company, MANAGE).await?;
        if self.0 == "order.transition" && prepare::text(v, "status") == "confirmed" {
            crate::authorize(c, s, company, CONFIRM).await?;
        }
        // The plan already owns this connection. Use the same narrow lock as
        // Purchasing; a second pool checkout would wait on our own company lock.
        let company_row=c.query_opt("SELECT status FROM companies WHERE tenant_id=$1 AND id=$2 AND deleted_at IS NULL FOR UPDATE",&[&s.tenant(),&company]).await?.ok_or(Error::NotFound)?;
        if company_row.get::<_, String>(0) == "archived" {
            return Err(prepare::invalid(
                "company_id",
                "This company is archived; nothing in it can change.",
            ));
        }
        let services = SERVICES.get().ok_or(bilibi_database::Error::Internal)?;
        prepare::prepare(c, s, v, self.0, &services.records, &services.parties).await
    }
}
impl WriteHandler for Write {
    fn owner(&self) -> &'static str {
        MODULE
    }
    fn preview<'a>(
        &'a self,
        c: &'a dyn OperationClient,
        s: &'a Scope,
        v: &'a Value,
    ) -> OperationFuture<'a> {
        Box::pin(async move { Ok(self.prepare(c, s, v).await?.review().into()) })
    }
    fn commit<'a>(
        &'a self,
        tx: &'a mut MutationTransaction<'_>,
        s: &'a Scope,
        v: &'a Value,
    ) -> OperationFuture<'a> {
        Box::pin(async move {
            let p = self.prepare(tx.sql(), s, v).await?;
            let changed = p.old.as_ref().is_some_and(|r| r.status != p.target);
            let record = store::apply(
                tx,
                s,
                p.company,
                &p.kind,
                &p.code,
                p.old,
                p.facts,
                &p.target,
                &p.reason,
                p.quotation,
            )
            .await?;
            if record.kind == "order" && changed {
                let services = SERVICES.get().ok_or(bilibi_database::Error::Internal)?;
                let lines = record.facts["lines"]
                    .as_array()
                    .ok_or(bilibi_database::Error::Internal)?;
                let subject = format!("order:{}", record.id);
                for (index, line) in lines.iter().enumerate() {
                    let key = format!("revision:{}:line:{}", record.revision, index + 1);
                    let payload = json!({"company_id":p.company,"order_id":record.id,"order_number":record.code,"revision":record.revision,"order_status":record.status,"quotation_id":record.facts["quotation_id"],"customer_kind":record.facts["customer_kind"],"customer_id":record.facts["customer_id"],"customer_code":record.facts["customer_code"],"currency_code":record.facts["currency_code"],"subtotal":record.facts["subtotal"],"tax_total":record.facts["tax_total"],"total":record.facts["total"],"part_index":index+1,"part_count":lines.len(),"lines":[line]});
                    services
                        .events
                        .record(
                            tx,
                            NewEvent {
                                name: STATUS_CHANGED,
                                subject: &subject,
                                key: &key,
                                payload: payload.clone(),
                            },
                        )
                        .await?;
                    if record.status == "confirmed" {
                        services
                            .events
                            .record(
                                tx,
                                NewEvent {
                                    name: CONFIRMED,
                                    subject: &subject,
                                    key: &key,
                                    payload,
                                },
                            )
                            .await?;
                    }
                }
            }
            Ok(json!({"id":record.id,"record":record.snapshot()}).into())
        })
    }
}
fn schema() -> Value {
    json!({"type":"object","required":["company_id"],"additionalProperties":false,"properties":{
 "company_id":{"type":"integer","minimum":1},"id":{"type":"integer","minimum":1},"revision":{"type":"integer","minimum":1},
 "code":{"type":"string","maxLength":50},"name":{"type":"string","maxLength":200},"unit":{"type":"string","maxLength":20},"default_price":{"type":"string","maxLength":30},"tax_code":{"type":"string","maxLength":50},"tax_rate":{"type":"string","maxLength":30},"active":{"type":"boolean"},
 "customer_kind":{"type":"string","enum":["company","person"]},"customer_code":{"type":"string","maxLength":255},"currency_code":{"type":"string","maxLength":3},"valid_until":{"type":"string","maxLength":10},"reference":{"type":"string","maxLength":200},"quotation_id":{"type":"integer","minimum":1},"status":{"type":"string","enum":["sent","accepted","rejected","expired","confirmed","cancelled"]},"reason":{"type":"string","maxLength":500},
 "lines":{"type":"array","minItems":1,"maxItems":100,"items":{"type":"object","additionalProperties":false,"properties":{"item_id":{"type":"integer","minimum":1},"description":{"type":"string","maxLength":500},"quantity":{"type":"string","maxLength":30},"unit":{"type":"string","maxLength":20},"unit_price":{"type":"string","maxLength":30},"tax_code":{"type":"string","maxLength":50},"tax_rate":{"type":"string","maxLength":30}}}}}})
}
macro_rules! op {($key:literal,$title:literal,$action:literal)=>{Operation{key:$key,title:$title,summary:$title,keywords:&["sales","commerce","quotation","order","catalog"],capability:MANAGE,input:schema,output:||json!({"type":"object"}),handler:Handler::SafeWrite(&Write($action)),channels:Channels::Every,step_up:&[]}};}
inventory::submit! {Contribution::new(DEFINITIONS,"commerce/sales.operations",Provenance{module:MODULE,source:file!(),line:line!()},&Definitions{
operations:&[
op!("commerce.sales.item.create","Create a catalog item","item.create"),
op!("commerce.sales.item.update","Update a catalog item","item.update"),
op!("commerce.sales.quotation.create","Create a quotation","quotation.create"),
op!("commerce.sales.quotation.transition","Change quotation status","quotation.transition"),
op!("commerce.sales.order.create","Create a sales order","order.create"),
op!("commerce.sales.order.transition","Confirm or cancel a sales order","order.transition")],guides:&[]})}
