//! Rules offers for one existing owner event and a private Core task.
use bilibi_activity::FollowUp;
use bilibi_agent_api::{Channels, Definitions, Handler, Operation, OperationClient, DEFINITIONS};
use bilibi_authz::{OfferedCapabilities, OFFERED_CAPABILITIES};
use bilibi_database::Error;
use bilibi_events::On;
use bilibi_grid::{Field, FieldType, GridContribution, Source, TableDecl};
use bilibi_module_registry::{Contribution, Provenance};
use bilibi_party_ids::OrganisationId;
use bilibi_rules::{Action, Trigger, ACTIONS, TRIGGERS};
use bilibi_tenancy::{ProofFuture, Scope};
use serde_json::{json, Value};

pub(crate) const MANAGE: &str = "commerce.sales.followup.manage";
const READ: &str = "commerce.sales.view";
const KEY: &str = "commerce.sales.order.follow_up";
const TABLE: &str = "commerce_rule_orders";
const SOURCE_KIND: &str = "commerce.sales.order";
const EVENT: &str = "commerce/sales.order_confirmed";
macro_rules! provenance {
    () => {
        Provenance {
            module: crate::MODULE,
            source: file!(),
            line: line!(),
        }
    };
}

fn check<'a>(
    client: &'a dyn OperationClient,
    scope: &'a Scope,
    input: &'a Value,
) -> ProofFuture<'a, OrganisationId> {
    Box::pin(async move {
        let id = input["record_id"].as_i64().ok_or(Error::Invalid)?;
        let company: i64 = client.query_opt(
            "SELECT company_id FROM commerce_sales_documents WHERE tenant_id=$1 AND id=$2 AND kind='order'",
            &[&scope.tenant(), &id],
        ).await?.ok_or(Error::NotFound)?.get(0);
        bilibi_company::authorize_work_target(client, scope, company, MANAGE).await?;
        bilibi_company::authorize_work_target(client, scope, company, READ).await?;
        bilibi_company::require_writable(client, scope, company, true)
            .await
            .map_err(|e| Error::Refused(e.to_string()))?;
        let row = client.query_opt(
            "SELECT revision,status FROM commerce_sales_documents WHERE tenant_id=$1 AND id=$2 AND company_id=$3 AND kind='order' FOR UPDATE",
            &[&scope.tenant(), &id, &company],
        ).await?.ok_or(Error::NotFound)?;
        if input["expected_revision"].as_i64() != Some(row.get::<_, i64>(0)) {
            return Err(Error::Conflict);
        }
        if row.get::<_, String>(1) != "confirmed" {
            return Err(Error::Refused(
                "The source record is no longer confirmed.".into(),
            ));
        }
        OrganisationId::new(company).map_err(|_| Error::Invalid)
    })
}
static HANDLER: FollowUp = FollowUp {
    module: crate::MODULE,
    kind: SOURCE_KIND,
    check,
};
inventory::submit! { Contribution::new(DEFINITIONS, "commerce/sales.follow_up", provenance!(), &Definitions {
    operations: &[Operation { key: KEY, title: "Create a follow-up task", summary: "Create a private task for a staff member about this confirmed record.", keywords: &["follow-up", "task"], capability: MANAGE, input: FollowUp::input, output: || json!({"type":"object"}), handler: Handler::SafeWrite(&HANDLER), channels: Channels::Every, step_up: &[] }], guides: &[],
}) }
inventory::submit! { Contribution::new(OFFERED_CAPABILITIES, "rules:commerce/sales", provenance!(), &OfferedCapabilities { principal: "rules", capabilities: &[READ, MANAGE] }) }
inventory::submit! { Contribution::new(bilibi_agent_api::job::PRINCIPAL_WRITES, KEY, provenance!(), &bilibi_agent_api::job::PrincipalWrite { principals: &["rules"] }) }
inventory::submit! { Contribution::new(TRIGGERS, EVENT, provenance!(), &Trigger { on: On { event: EVENT, versions: &[1] }, title: "A sales order is confirmed", table: TABLE, subject: "/order_id", logical_event: Some("/revision"), version: "revision" }) }
inventory::submit! { Contribution::new(ACTIONS, KEY, provenance!(), &Action { title: "Create a follow-up task", table: TABLE, subject: "record_id", version: "expected_revision", records: &[], approval_capabilities: &[bilibi_activity::MANAGE] }) }

static GRID: GridContribution = GridContribution {
    tables: &[TableDecl::new(TABLE, "Sales orders", READ,
        Source::tenant("SELECT id,company_id,revision AS revision,status,id::text AS label,facts->>'total' AS total,facts->>'currency_code' AS currency FROM commerce_sales_documents WHERE tenant_id=$1 AND kind='order' AND ($2::bigint=0 OR company_id IN (SELECT d.company_id FROM user_desks d JOIN users u ON u.tenant_id=d.tenant_id AND u.person_id=d.person_id WHERE d.tenant_id=$1 AND u.id=$2 AND u.kind='staff' AND u.deleted_at IS NULL))"), "id", &[
            Field::new("id", "ID", FieldType::Integer).protected(),
            Field::new("company_id", "Company ID", FieldType::Integer).protected(),
            Field::new("revision", "Revision", FieldType::Integer).protected(),
            Field::new("status", "Status", FieldType::String).protected(),
            Field::new("label", "Number", FieldType::String).protected(),
            Field::new("total", "Total", FieldType::Decimal),
            Field::new("currency", "Currency", FieldType::String),
        ]).label_field("label")], record_fields: &[], links: &[],
};
bilibi_grid::contribute!("commerce/sales", "commerce/sales.rules", &GRID);
