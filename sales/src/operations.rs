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

pub(crate) const DELIVERED: &str = "commerce/sales.delivery_delivered";
pub(crate) const CONFIRMED: &str = "commerce/sales.order_confirmed";
pub(crate) const STATUS_CHANGED: &str = "commerce/sales.order_status_changed";
struct Services {
    events: Arc<Events>,
    records: Arc<Records>,
    parties: Arc<PartyProofs>,
    currencies: Arc<bilibi_currency::Currencies>,
    products: Arc<bilibi_product::Products>,
}
static SERVICES: OnceLock<Services> = OnceLock::new();
pub(crate) fn bind(
    events: Arc<Events>,
    records: Arc<Records>,
    parties: Arc<PartyProofs>,
    currencies: Arc<bilibi_currency::Currencies>,
    products: Arc<bilibi_product::Products>,
) -> bilibi_module_registry::Result<()> {
    SERVICES
        .set(Services {
            events,
            records,
            parties,
            currencies,
            products,
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
        tx: Option<&MutationTransaction<'_>>,
    ) -> bilibi_agent_api::Result<prepare::Prepared> {
        let company = prepare::integer(v, "company_id")?;
        crate::authorize(c, s, company, MANAGE).await?;
        if self.0 == "order.transition" && prepare::text(v, "status") == "confirmed" {
            crate::authorize(c, s, company, CONFIRM).await?;
        }
        // The plan already owns this connection. Use the same narrow lock as
        // Purchasing; a second pool checkout would wait on our own company lock.
        let company_row=c.query_opt("SELECT status FROM companies WHERE tenant_id=$1 AND id=$2 AND deleted_at IS NULL FOR NO KEY UPDATE",&[&s.tenant(),&company]).await?.ok_or(Error::NotFound)?;
        if company_row.get::<_, String>(0) == "archived" {
            return Err(prepare::invalid(
                "company_id",
                "This company is archived; nothing in it can change.",
            ));
        }
        let services = SERVICES.get().ok_or(bilibi_database::Error::Internal)?;
        prepare::prepare(
            c,
            s,
            v,
            self.0,
            &services.records,
            &services.parties,
            &services.currencies,
            &services.products,
            tx,
        )
        .await
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
        Box::pin(async move { Ok(self.prepare(c, s, v, None).await?.review().into()) })
    }
    fn commit<'a>(
        &'a self,
        tx: &'a mut MutationTransaction<'_>,
        s: &'a Scope,
        v: &'a Value,
    ) -> OperationFuture<'a> {
        Box::pin(async move {
            let company = prepare::integer(v, "company_id")?;
            crate::authorize(tx.sql(), s, company, MANAGE).await?;
            bilibi_company::require_writable(tx.sql(), s, company, true)
                .await
                .map_err(|e| match e {
                    bilibi_company::WriteError::Archived => prepare::invalid(
                        "company_id",
                        "This company is archived; nothing in it can change.",
                    ),
                    bilibi_company::WriteError::NotFound => Error::NotFound,
                    bilibi_company::WriteError::Access(error) => error.into(),
                    _ => bilibi_database::Error::Internal.into(),
                })?;
            let p = self.prepare(tx.sql(), s, v, Some(tx)).await?;
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
            if record.kind == "delivery" && record.status == "delivered" && changed {
                let services = SERVICES.get().ok_or(bilibi_database::Error::Internal)?;
                let lines = record.facts["lines"]
                    .as_array()
                    .ok_or(bilibi_database::Error::Internal)?;
                for (index, line) in lines.iter().enumerate() {
                    services.events.record(tx, NewEvent {
                        name: DELIVERED,
                        subject: &format!("delivery:{}", record.id),
                        key: &format!("delivery:{}:revision:{}:delivered:line:{}", record.id, record.revision, index+1),
                        payload: json!({"company_id":p.company,"delivery_id":record.id,"delivery_number":record.code,"revision":record.revision,"status":record.status,"order_id":record.facts["order_id"],"order_number":record.facts["order_number"],"order_revision":record.facts["order_revision"],"customer_kind":record.facts["customer_kind"],"customer_id":record.facts["customer_id"],"customer_code":record.facts["customer_code"],"reference":record.facts["reference"],"part_index":index+1,"part_count":lines.len(),"lines":[line]}),
                    }).await?;
                }
            }
            if record.kind == "order" && changed {
                let services = SERVICES.get().ok_or(bilibi_database::Error::Internal)?;
                let lines = record.facts["lines"]
                    .as_array()
                    .ok_or(bilibi_database::Error::Internal)?;
                let subject = format!("order:{}", record.id);
                for (index, line) in lines.iter().enumerate() {
                    let key = format!(
                        "order:{}:revision:{}:status:line:{}",
                        record.id,
                        record.revision,
                        index + 1
                    );
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
                                    key: &format!(
                                        "order:{}:revision:{}:confirmed:line:{}",
                                        record.id,
                                        record.revision,
                                        index + 1
                                    ),
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
// Base Agent API accepts its documented schema dialect, not full JSON Schema.
// Object fields are closed by default; cross-field and array bounds live in prepare.
fn schema(action: &str) -> Value {
    if action == "delivery.create" {
        return json!({"type":"object","required":["company_id"],"properties":{
            "company_id":{"type":"integer","minimum":1},"order_id":{"type":"integer","minimum":1},"order_number":{"type":"integer","minimum":1},"reference":{"type":"string","maxLength":200},
            "lines":{"type":"array","items":{"type":"object","required":["order_line_number","quantity"],"properties":{"order_line_number":{"type":"integer","minimum":1},"quantity":{"type":"string","maxLength":30}}}}
        }});
    }
    if action == "delivery.transition" {
        return json!({"type":"object","required":["company_id","id","revision","status"],"properties":{
            "company_id":{"type":"integer","minimum":1},"id":{"type":"integer","minimum":1},"revision":{"type":"integer","minimum":1},"status":{"type":"string","enum":["shipped","delivered","cancelled"]},"reason":{"type":"string","maxLength":500}
        }});
    }
    let line = json!({"type":"object","properties":{"product_id":{"type":"integer","minimum":1},"description":{"type":"string","maxLength":500},"quantity":{"type":"string","maxLength":30},"unit":{"type":"string","maxLength":80},"unit_price":{"type":"string","maxLength":30},"tax_code":{"type":"string","maxLength":50},"tax_rate":{"type":"string","maxLength":30}}});
    let mut schema = json!({"type":"object","required":["company_id"],"properties":{
 "company_id":{"type":"integer","minimum":1},"id":{"type":"integer","minimum":1},"revision":{"type":"integer","minimum":1},
 "price_on":{"type":"string","maxLength":10},
 "customer_kind":{"type":"string","enum":["company","person"]},"customer_code":{"type":"string","maxLength":255},"currency_code":{"type":"string","maxLength":3},"valid_until":{"type":"string","maxLength":10},"reference":{"type":"string","maxLength":200},"quotation_id":{"type":"integer","minimum":1},"quotation_number":{"type":"integer","minimum":1},"status":{"type":"string","enum":["sent","accepted","rejected","expired","confirmed","cancelled"]},"reason":{"type":"string","maxLength":500},
 "lines":{"type":"array","items":line}}});
    let (allowed, required): (&[&str], &[&str]) = match action {
        "quotation.create" => (
            &[
                "company_id",
                "customer_kind",
                "customer_code",
                "currency_code",
                "price_on",
                "valid_until",
                "reference",
                "lines",
            ],
            &[
                "company_id",
                "customer_kind",
                "customer_code",
                "currency_code",
                "valid_until",
                "lines",
            ],
        ),
        "order.create" => (
            &[
                "company_id",
                "customer_kind",
                "customer_code",
                "currency_code",
                "price_on",
                "reference",
                "lines",
                "quotation_id",
                "quotation_number",
            ],
            &["company_id"],
        ),
        _ => (
            &["company_id", "id", "revision", "status", "reason"],
            &["company_id", "id", "revision", "status"],
        ),
    };
    if let Some(properties) = schema["properties"].as_object_mut() {
        properties.retain(|key, _| allowed.contains(&key.as_str()));
    }
    schema["required"] = json!(required);
    if action.ends_with("transition") {
        schema["properties"]["status"]["enum"] = if action.starts_with("quotation") {
            json!(["sent", "accepted", "rejected", "expired"])
        } else {
            json!(["confirmed", "cancelled"])
        };
    }
    schema
}
macro_rules! op {($key:literal,$title:literal,$action:literal)=>{Operation{key:$key,title:$title,summary:$title,keywords:&["sales","commerce","quotation","order","catalog"],capability:MANAGE,input:||schema($action),output:||json!({"type":"object"}),handler:Handler::SafeWrite(&Write($action)),channels:Channels::Every,step_up:&[]}};}
inventory::submit! {Contribution::new(DEFINITIONS,"commerce/sales.operations",Provenance{module:MODULE,source:file!(),line:line!()},&Definitions{
operations:&[
op!("commerce.sales.delivery.create","Create a delivery order","delivery.create"),
op!("commerce.sales.delivery.transition","Ship, deliver or cancel a delivery order","delivery.transition"),
op!("commerce.sales.quotation.create","Create a quotation","quotation.create"),
op!("commerce.sales.quotation.transition","Change quotation status","quotation.transition"),
op!("commerce.sales.order.create","Create a sales order","order.create"),
op!("commerce.sales.order.transition","Confirm or cancel a sales order","order.transition")],guides:&[]})}
