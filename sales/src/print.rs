//! Base Artifacts owns rendering, private storage, audit and download policy.
use crate::{Sales, VIEW};
use askama::Template;
use bilibi_artifacts::{Artifacts, HtmlFuture, HtmlOwner, Operation, Owner, Reference};
use bilibi_database::{Error, Result};
use bilibi_party_ids::OrganisationId;
use bilibi_tenancy::{ProofClient, ProofFuture, Scope};
use bilibi_ui::download::{Download, DownloadFuture, DownloadRequest, FileBody};
use serde_json::{json, Value};
use std::sync::Arc;
pub(crate) const SERVICE: &str = "commerce/sales.delivery-print";
pub(crate) struct DeliveryPrint {
    sales: Arc<Sales>,
    artifacts: Arc<Artifacts>,
}
impl DeliveryPrint {
    pub fn new(sales: Arc<Sales>, artifacts: Arc<Artifacts>) -> Self {
        Self { sales, artifacts }
    }
}
fn id(reference: Option<&Reference>) -> Result<i64> {
    reference
        .filter(|r| r.kind == "delivery-order")
        .and_then(|r| r.subject.parse().ok())
        .filter(|id| *id > 0)
        .ok_or(Error::NotFound)
}
impl Owner for DeliveryPrint {
    fn owner_id(&self) -> &'static str {
        "commerce/sales"
    }
    fn authorize<'a>(
        &'a self,
        c: &'a dyn ProofClient,
        s: &'a Scope,
        company: OrganisationId,
        operation: Operation,
        reference: Option<&'a Reference>,
    ) -> ProofFuture<'a, ()> {
        Box::pin(async move {
            if !matches!(operation, Operation::Create | Operation::Read) {
                return Err(Error::Forbidden);
            }
            bilibi_company::authorize_work_target(c, s, company.get(), VIEW).await?;
            let id = id(reference)?;
            c.query_opt("SELECT id FROM commerce_sales_documents WHERE tenant_id=$1 AND company_id=$2 AND kind='delivery' AND id=$3", &[&s.tenant(),&company.get(),&id]).await?.ok_or(Error::NotFound)?;
            Ok(())
        })
    }
}
impl HtmlOwner for DeliveryPrint {
    fn render_html<'a>(
        &'a self,
        s: &'a Scope,
        company: OrganisationId,
        reference: &'a Reference,
        _: &'a Value,
    ) -> HtmlFuture<'a> {
        Box::pin(async move {
            let record = self
                .sales
                .get(s, company.get(), "delivery", id(Some(reference))?)
                .await?;
            let c = self.sales.db.get().await?;
            let issuer=c.query_opt("SELECT name FROM companies WHERE tenant_id=$1 AND id=$2 AND deleted_at IS NULL", &[&s.tenant(),&company.get()]).await?.ok_or(Error::NotFound)?.try_get(0)?;
            let lines = record.facts["lines"]
                .as_array()
                .ok_or(Error::Internal)?
                .iter()
                .map(|l| PrintLine {
                    number: l["order_line_number"].to_string(),
                    description: crate::prepare::text(l, "description").into(),
                    unit: crate::prepare::text(l, "unit").into(),
                    quantity: crate::prepare::text(l, "quantity").into(),
                })
                .collect();
            DeliveryHtml {
                issuer,
                number: record.code,
                status: record.status,
                customer: crate::prepare::text(&record.facts, "customer_code").into(),
                order: crate::prepare::text(&record.facts, "order_number").into(),
                reference: crate::prepare::text(&record.facts, "reference").into(),
                lines,
            }
            .render()
            .map_err(|_| bilibi_artifacts::Error::Invalid)
        })
    }
}
impl Download<Scope, Error> for DeliveryPrint {
    fn serve<'a>(&'a self, r: DownloadRequest<'a, Scope>) -> DownloadFuture<'a, Error> {
        Box::pin(async move {
            let company = r.scope.current_company().ok_or(Error::Forbidden)?;
            let id = r
                .path
                .get("id")
                .and_then(|id| id.parse::<i64>().ok())
                .filter(|id| *id > 0)
                .ok_or(Error::NotFound)?;
            self.sales
                .get(r.scope, company.get(), "delivery", id)
                .await?;
            let mut c = self.sales.db.get().await?;
            let documents = self.artifacts.for_owner(r.scope, company, self);
            let metadata = documents
                .render_inline(
                    &mut c,
                    Reference {
                        subject: id.to_string(),
                        kind: "delivery-order".into(),
                    },
                    &json!({}),
                )
                .await
                .map_err(|_| Error::Internal)?;
            let document = documents
                .read(&mut c, &metadata.id)
                .await
                .map_err(|_| Error::Internal)?;
            Ok(FileBody::inline(
                document.bytes,
                "application/pdf",
                &format!("delivery-order-{id}.pdf"),
            ))
        })
    }
}
#[derive(Template)]
#[template(path = "delivery-print.html")]
struct DeliveryHtml {
    issuer: String,
    number: String,
    status: String,
    customer: String,
    order: String,
    reference: String,
    lines: Vec<PrintLine>,
}
struct PrintLine {
    number: String,
    description: String,
    quantity: String,
    unit: String,
}
