use crate::{Sales, CONFIRM, MANAGE, MODULE, SERVICE, VIEW};
use bilibi_authz::{CapabilityDefinition as C, RoleDefinition, VocabularyContribution, VOCABULARY};
use bilibi_database::Db;
use bilibi_events::{EventType, Events, EVENTS};
use bilibi_menu::{MenuItem, Requirement};
use bilibi_module_registry::{
    contributions::{
        Access, CompanyScope, Method, PageWrites, Pending, Route, PAGE_WRITES, ROUTES,
    },
    Contribution, Layer, Module, Provenance, Service,
};
use bilibi_tenancy::Scope;
use bilibi_ui::page::PageService;
use std::sync::Arc;
pub(crate) const PAGES: &str = "commerce/sales.pages";
macro_rules! provenance {
    () => {
        Provenance {
            module: MODULE,
            source: file!(),
            line: line!(),
        }
    };
}
inventory::submit! {Module{id:MODULE,layer:Layer::Domains,
dependencies:&["base/agent_api","base/artifacts","base/authz","base/database","base/datetime","base/events","base/grid","base/locale","base/menu","base/records","base/party_ids","base/tenancy","base/ui","core/company"],
migrations:&[("commerce-sales-001-sales",include_str!("../migrations/001-sales.sql")),("commerce-sales-002-deliveries",include_str!("../migrations/002-deliveries.sql"))],
services:&[
Service{id:SERVICE,factory:|services|{
crate::operations::bind(services.get::<Events>(bilibi_events::SERVICE)?,services.get(bilibi_records::SERVICE)?,services.get("base/tenancy.parties")?)?;
Ok(Arc::new(Sales::new(services.get::<Db>("base/database.pool")?.as_ref().clone())))
}},
Service{id:crate::print::SERVICE,factory:|services|Ok(Arc::new(bilibi_ui::download::DownloadService::<Scope,bilibi_database::Error>(Arc::new(crate::print::DeliveryPrint::new(services.get::<Sales>(SERVICE)?,services.get("base/artifacts.store")?)))))},
Service{id:PAGES,factory:|services|Ok(Arc::new(PageService::<Scope,bilibi_database::Error>(Arc::new(crate::web::Pages::new(services.get::<Sales>(SERVICE)?,services.get(bilibi_agent_api::page::SERVICE)?,services.get(bilibi_grid::PAGE_VIEWS)?,services.get("base/datetime.display")?,services.get("base/locale.locales")?)))))},
]}}
inventory::submit! {Contribution::new(VOCABULARY,"commerce/sales.vocabulary",provenance!(),&VocabularyContribution{
domains:&["commerce"],verbs:&["confirm"],capabilities:&[C::tenant(VIEW),C::tenant(MANAGE),C::tenant(CONFIRM)],roles:&[RoleDefinition{code:"tenant_owner",name:"Tenant Owner",description:"Contributed system role; reconciled explicitly",grant_all:false,capabilities:&[VIEW,MANAGE,CONFIRM]}],system_principals:&[]})}
inventory::submit! {Contribution::new(EVENTS,crate::operations::CONFIRMED,provenance!(),&EventType{version:1})}
inventory::submit! {Contribution::new(EVENTS,crate::operations::STATUS_CHANGED,provenance!(),&EventType{version:1})}
inventory::submit! {Contribution::new(PAGE_WRITES,PAGES,provenance!(),&PageWrites{service:PAGES,contracts:&[
("create_delivery","commerce.sales.delivery.create"),("delivery_transition","commerce.sales.delivery.transition"),("confirm_action","commerce.sales.delivery.transition"),
("create_item","commerce.sales.item.create"),("update_item","commerce.sales.item.update"),
("create_quotation","commerce.sales.quotation.create"),("quotation_transition","commerce.sales.quotation.transition"),
("create_order","commerce.sales.order.create"),("order_transition","commerce.sales.order.transition"),("confirm_action","commerce.sales.quotation.transition"),("confirm_action","commerce.sales.order.transition")],pending:Pending::None})}
macro_rules! route {($id:literal,$path:literal,$op:literal)=>{inventory::submit!{Contribution::new(ROUTES,$id,provenance!(),&Route{company_scope:CompanyScope::Company,method:Method::Get,path:$path,service:PAGES,operation:$op,access:Access::Capability(VIEW),framed:true})}};}
route!("commerce/sales.items", "/commerce/items", "items");
route!("commerce/sales.item", "/commerce/items/{id}", "item");
route!(
    "commerce/sales.quotations",
    "/commerce/quotations",
    "quotations"
);
route!(
    "commerce/sales.quotation",
    "/commerce/quotations/{id}",
    "quotation"
);
route!("commerce/sales.orders", "/commerce/sales-orders", "orders");
route!(
    "commerce/sales.order",
    "/commerce/sales-orders/{id}",
    "order"
);
bilibi_menu::contribute!(
    "commerce/sales",
    MenuItem::container("commerce", "Commerce")
        .icon("clipboard-document-list")
        .order(54)
);
bilibi_menu::contribute!(
    "commerce/sales",
    MenuItem::page("commerce.items", "Products", "/commerce/items")
        .company_scoped()
        .under("commerce")
        .requires(Requirement::Capability(VIEW))
        .order(10)
);
bilibi_menu::contribute!(
    "commerce/sales",
    MenuItem::page("commerce.quotations", "Quotations", "/commerce/quotations")
        .company_scoped()
        .under("commerce")
        .requires(Requirement::Capability(VIEW))
        .order(20)
);
bilibi_menu::contribute!(
    "commerce/sales",
    MenuItem::page("commerce.orders", "Sales orders", "/commerce/sales-orders")
        .company_scoped()
        .under("commerce")
        .requires(Requirement::Capability(VIEW))
        .order(30)
);

inventory::submit! {Contribution::new(EVENTS,crate::operations::DELIVERED,provenance!(),&EventType{version:1})}
route!(
    "commerce/sales.deliveries",
    "/commerce/delivery-orders",
    "deliveries"
);
route!(
    "commerce/sales.delivery",
    "/commerce/delivery-orders/{id}",
    "delivery"
);
bilibi_menu::contribute!(
    "commerce/sales",
    MenuItem::page(
        "commerce.deliveries",
        "Delivery orders",
        "/commerce/delivery-orders"
    )
    .company_scoped()
    .under("commerce")
    .requires(Requirement::Capability(VIEW))
    .order(40)
);
inventory::submit! {Contribution::new(bilibi_module_registry::contributions::DOWNLOADS,"commerce/sales.delivery-pdf",provenance!(),&bilibi_module_registry::contributions::Download{path:"/commerce/delivery-orders/{id}/pdf",service:crate::print::SERVICE,operation:"delivery_pdf",access:Access::Capability(VIEW)})}
