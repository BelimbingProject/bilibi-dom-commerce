use crate::{contributions::PAGES, prepare::text, Record, Sales, CONFIRM, MANAGE, VIEW};
use askama::Template;
use bilibi_agent_api::page::{Call, PageContracts};
use bilibi_database::{Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{
        LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState, MountFuture, Page, PageBody,
        PageFuture, PageRequest,
    },
    *,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, LazyLock},
};

pub(crate) struct Pages {
    sales: Arc<Sales>,
    contracts: Arc<PageContracts>,
}
impl Pages {
    pub fn new(sales: Arc<Sales>, contracts: Arc<PageContracts>) -> Self {
        Self { sales, contracts }
    }
}
fn title(k: &str) -> &'static str {
    match k {
        "item" => "Products",
        "quotation" => "Quotations",
        _ => "Sales orders",
    }
}
fn path(k: &str) -> &'static str {
    match k {
        "item" => "/commerce/items",
        "quotation" => "/commerce/quotations",
        _ => "/commerce/sales-orders",
    }
}
fn kind(op: &str) -> &'static str {
    match op {
        "item" | "items" => "item",
        "quotation" | "quotations" => "quotation",
        _ => "order",
    }
}
fn render(t: &impl Template) -> Result<Html> {
    Html::render(t).map_err(|_| Error::Internal)
}
static SPEC: LazyLock<Arc<ListSpec>> = LazyLock::new(|| {
    ListSpec {
        filters: BTreeMap::from([(
            "status".into(),
            Filter::one_of(
                &[
                    "",
                    "draft",
                    "sent",
                    "accepted",
                    "rejected",
                    "expired",
                    "confirmed",
                    "cancelled",
                ],
                "",
            ),
        )]),
        ..ListSpec::default()
    }
    .shared()
    .expect("sales list spec")
});
fn list(q: &HashMap<String, String>) -> Result<ListState> {
    ListState::parse(q, SPEC.clone()).map_err(|_| Error::Invalid)
}
impl Page<Scope, Error> for Pages {
    fn title(&self, op: &str) -> Option<&'static str> {
        Some(title(kind(op)))
    }
    fn render<'a>(&'a self, r: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut state = State::new(self, r.operation, r.path, r.query, r.framed)?;
            state.load(r.scope.ok_or(Error::Unauthorized)?).await?;
            Ok(PageBody {
                html: state.view()?.to_string(),
                live: true,
                title: Some(state.heading()),
            })
        })
    }
    fn mount<'a>(&'a self, r: PageRequest<'a, Scope>) -> MountFuture<'a, Scope, Error> {
        Box::pin(async move {
            Ok(Some(
                Box::new(State::new(self, r.operation, r.path, r.query, r.framed)?)
                    as Box<dyn LiveState<Scope, Error>>,
            ))
        })
    }
}
struct State {
    sales: Arc<Sales>,
    contracts: Arc<PageContracts>,
    kind: &'static str,
    id: Option<i64>,
    framed: bool,
    list: ListState,
    columns: ListColumns,
    rows: Vec<Record>,
    record: Option<Record>,
    history: Vec<Value>,
    items: Vec<Record>,
    manage: bool,
    confirm: bool,
    editing: bool,
    form: HashMap<String, String>,
    line_count: usize,
    flash: Option<(StatusKind, String)>,
    errors: BTreeMap<String, Vec<String>>,
    commits: CommitStates,
    pending: Option<(String, String, Value)>,
    redirect: Option<String>,
    total: u64,
}
impl State {
    fn new(
        p: &Pages,
        op: &str,
        params: &HashMap<String, String>,
        q: &HashMap<String, String>,
        framed: bool,
    ) -> Result<Self> {
        let k = kind(op);
        let id = params::positive_id(params.get("id").map(String::as_str));
        let cols = if k == "item" {
            vec![
                ("code", "Code"),
                ("name", "Name"),
                ("unit", "Unit"),
                ("price", "Default price"),
                ("tax", "Tax code"),
            ]
        } else {
            vec![
                ("code", "Number"),
                ("customer", "Customer"),
                ("total", "Total"),
                ("status", "Status"),
            ]
        };
        Ok(Self {
            sales: p.sales.clone(),
            contracts: p.contracts.clone(),
            kind: k,
            id,
            framed,
            list: list(q)?,
            columns: ListColumns::mount(
                "commerce-table",
                cols.into_iter()
                    .map(|(k, l)| BuiltinColumn::new(k, l))
                    .collect(),
            )
            .map_err(|_| Error::Internal)?,
            rows: vec![],
            record: None,
            history: vec![],
            items: vec![],
            manage: false,
            confirm: false,
            editing: false,
            form: HashMap::new(),
            line_count: 1,
            flash: None,
            errors: BTreeMap::new(),
            commits: CommitStates::default(),
            pending: None,
            redirect: None,
            total: 0,
        })
    }
    fn heading(&self) -> String {
        self.record.as_ref().map_or_else(
            || title(self.kind).into(),
            |r| {
                format!(
                    "{} {}",
                    match self.kind {
                        "item" => "Product",
                        "quotation" => "Quotation",
                        _ => "Sales order",
                    },
                    r.code
                )
            },
        )
    }
    async fn load(&mut self, s: &Scope) -> Result<()> {
        let company = s.current_company().map_or(0, |c| c.get());
        let c = self.sales.db.get().await?;
        crate::authorize(&**c, s, company, VIEW).await?;
        self.manage = crate::authorize(&**c, s, company, MANAGE).await.is_ok();
        self.confirm = crate::authorize(&**c, s, company, CONFIRM).await.is_ok();
        drop(c);
        if let Some(id) = self.id {
            self.record = Some(self.sales.get(s, company, self.kind, id).await?);
            if self.kind != "item" {
                self.history = self.sales.history(s, company, id).await?;
            }
        } else {
            let status = self.list.filters.get("status").map_or("", String::as_str);
            self.rows = self
                .sales
                .list(
                    s,
                    company,
                    self.kind,
                    &self.list.search,
                    status,
                    self.list.page_size as i64,
                    self.list.offset().min(i64::MAX as u64) as i64,
                )
                .await?;
            let c = self.sales.db.get().await?;
            self.total = crate::store::count(
                &**c,
                s.tenant(),
                company,
                self.kind,
                &self.list.search,
                status,
            )
            .await? as u64;
            self.columns.load(&self.rows, |r| r.id.to_string());
        }
        if self.manage && self.kind != "item" {
            self.items = self.sales.list(s, company, "item", "", "", 100, 0).await?;
        }
        Ok(())
    }
    fn field(&self, key: &str, label: &str) -> Field {
        Field::in_form("form", key)
            .label(label)
            .value(self.form.get(key).cloned().unwrap_or_default())
            .errors(self.errors.get(key).cloned().unwrap_or_default())
    }
    fn fields(&self) -> Vec<Field> {
        if self.kind == "item" {
            return [
                ("code", "Code"),
                ("name", "Name"),
                ("unit", "Unit"),
                ("default_price", "Default price"),
                ("tax_code", "Tax code"),
                ("tax_rate", "Tax rate (%)"),
            ]
            .iter()
            .map(|(k, l)| self.field(k, l))
            .collect();
        }
        let mut fields = vec![
            self.field("customer_kind", "Customer type")
                .kind(InputKind::Select)
                .options(vec![
                    Choice::new("Company", "company"),
                    Choice::new("Person", "person"),
                ]),
            self.field("customer_code", "Customer code")
                .hint("Use the code from Core Companies or Contacts."),
            self.field("currency_code", "Currency code"),
            self.field("reference", "Reference"),
        ];
        if self.kind == "quotation" {
            fields.push(
                self.field("valid_until", "Valid until")
                    .kind(InputKind::Date),
            );
        }
        if self.kind == "order" {
            fields.push(self.field("quotation_id","Accepted quotation ID").hint("Leave blank to enter an order directly. A quotation replaces the customer and lines entered below."));
        }
        fields
    }
    fn line_fields(&self) -> Vec<Vec<Field>> {
        (0..self.line_count)
            .map(|i| {
                let mut fields = vec![self
                    .field(&format!("line_{i}_item_id"), "Product")
                    .kind(InputKind::Select)
                    .prompt("Typed line")
                    .options(
                        self.items
                            .iter()
                            .filter(|r| r.facts["active"] != false)
                            .map(|r| {
                                Choice::new(
                                    format!("{} · {}", r.code, text(&r.facts, "name")),
                                    r.id.to_string(),
                                )
                            })
                            .collect(),
                    )];
                for (k, l) in [
                    ("description", "Description"),
                    ("quantity", "Quantity"),
                    ("unit", "Unit"),
                    ("unit_price", "Unit price"),
                    ("tax_code", "Tax code"),
                    ("tax_rate", "Tax rate (%)"),
                ] {
                    fields.push(self.field(&format!("line_{i}_{k}"), l));
                }
                fields
            })
            .collect()
    }
    fn input(&self, s: &Scope) -> Value {
        let mut v = json!({"company_id":s.current_company().map_or(0,|c|c.get())});
        for (k, val) in &self.form {
            if !k.starts_with("line_") && !val.is_empty() {
                v[k] = json!(val);
            }
        }
        if self.kind == "order" {
            if let Some(q) = self.form.get("quotation_id").filter(|s| !s.is_empty()) {
                v["quotation_id"] = json!(q.parse::<i64>().unwrap_or(0));
            }
        }
        if self.kind != "item" {
            let lines = (0..self.line_count)
                .filter_map(|i| {
                    let mut line = json!({});
                    for k in [
                        "description",
                        "quantity",
                        "unit",
                        "unit_price",
                        "tax_code",
                        "tax_rate",
                    ] {
                        line[k] = json!(self
                            .form
                            .get(&format!("line_{i}_{k}"))
                            .cloned()
                            .unwrap_or_default());
                    }
                    if let Some(item) = self
                        .form
                        .get(&format!("line_{i}_item_id"))
                        .filter(|s| !s.is_empty())
                    {
                        line["item_id"] = json!(item.parse::<i64>().unwrap_or(0));
                    }
                    if text(&line, "description").is_empty()
                        && line["item_id"].is_null()
                        && text(&line, "quantity").is_empty()
                    {
                        None
                    } else {
                        Some(line)
                    }
                })
                .collect::<Vec<_>>();
            v["lines"] = json!(lines);
        }
        v
    }
    async fn submit(
        &mut self,
        s: &Scope,
        credential: &str,
        event: &str,
        key: &'static str,
        input: Value,
    ) -> Result<()> {
        let call = Call {
            scope: s,
            credential,
            service: PAGES,
            event,
            contract: key,
        };
        match self.contracts.submit(&call, input).await {
            Ok(out) => {
                self.flash = Some((StatusKind::Success, "Saved.".into()));
                self.editing = false;
                self.errors.clear();
                self.form.clear();
                if self.id.is_none() {
                    if let Some(id) = out["id"].as_i64().or_else(|| out["result"]["id"].as_i64()) {
                        self.redirect = Some(format!("{}/{id}", path(self.kind)));
                    }
                }
            }
            Err(bilibi_agent_api::Error::InvalidInput(fields)) => {
                self.errors = fields;
                self.flash = Some((
                    StatusKind::Error,
                    "The record was not saved. Check the fields below.".into(),
                ));
            }
            Err(e) => {
                self.flash = Some((StatusKind::Error, format!("The record was not saved. {e}")));
            }
        }
        Ok(())
    }
    async fn event(&mut self, r: &LiveRequest<'_, Scope>, v: &Value) -> Result<()> {
        let event = text(v, "type");
        match event {
            "filters" => self.list.apply_filters(&params::form_values(v, "filters")),
            "page" => self.list.put_page(Some(text(v, "page"))),
            "grid" => {
                self.columns.handle(&params::string_values(v));
            }
            "clear-flash" => self.flash = None,
            "new" => {
                self.editing = true;
                self.form.clear();
                self.errors.clear();
                self.line_count = 1;
            }
            "close" => {
                self.editing = false;
                self.pending = None;
            }
            "draft" => {
                self.form = posted(v);
            }
            "add_line" => {
                if self.line_count < 100 {
                    self.line_count += 1;
                }
            }
            "create_item" | "create_quotation" | "create_order" => {
                self.form = posted(v);
                let key = match self.kind {
                    "item" => "commerce.sales.item.create",
                    "quotation" => "commerce.sales.quotation.create",
                    _ => "commerce.sales.order.create",
                };
                self.submit(r.scope, r.credential, event, key, self.input(r.scope))
                    .await?;
            }
            "update_item" => {
                if let Some(record) = &self.record {
                    let field = text(v, "field");
                    let value = text(v, "value");
                    if !["name", "unit", "default_price", "tax_code", "tax_rate"].contains(&field) {
                        return Ok(());
                    }
                    let mut input = record.facts.clone();
                    input["company_id"] = json!(r.scope.current_company().map_or(0, |c| c.get()));
                    input["id"] = json!(record.id);
                    input["revision"] = json!(text(v, "version").parse::<i64>().unwrap_or(0));
                    input[field] = json!(value);
                    let call = Call {
                        scope: r.scope,
                        credential: r.credential,
                        service: PAGES,
                        event,
                        contract: "commerce.sales.item.update",
                    };
                    let result = self.contracts.submit(&call, input).await;
                    self.commits.put(
                        field,
                        match result {
                            Ok(_) => CommitStatus::Saved,
                            Err(e) => CommitStatus::Error(e.to_string()),
                        },
                    );
                }
            }
            "quotation_transition" | "order_transition" => {
                if let Some(record) = &self.record {
                    let status = text(v, "form[status]");
                    let reason = text(v, "form[reason]");
                    let input = json!({"company_id":r.scope.current_company().map_or(0,|c|c.get()),"id":record.id,"revision":record.revision,"status":status,"reason":reason});
                    let key = if self.kind == "quotation" {
                        "commerce.sales.quotation.transition"
                    } else {
                        "commerce.sales.order.transition"
                    };
                    let call = Call {
                        scope: r.scope,
                        credential: r.credential,
                        service: PAGES,
                        event,
                        contract: key,
                    };
                    match self.contracts.preview(&call, input.clone()).await {
                        Ok(review) if review["valid"] == true => {
                            self.pending = Some((event.into(), key.into(), input))
                        }
                        Ok(review) => {
                            self.flash = Some((
                                StatusKind::Error,
                                format!("This change was refused: {}", review["results"]),
                            ))
                        }
                        Err(e) => self.flash = Some((StatusKind::Error, e.to_string())),
                    };
                }
            }
            "confirm_action" => {
                if let Some((event, key, input)) = self.pending.take() {
                    let key = if key == "commerce.sales.quotation.transition" {
                        "commerce.sales.quotation.transition"
                    } else {
                        "commerce.sales.order.transition"
                    };
                    let call = Call {
                        scope: r.scope,
                        credential: r.credential,
                        service: PAGES,
                        event: &event,
                        contract: key,
                    };
                    match self.contracts.commit(&call, input).await {
                        Ok(_) => self.flash = Some((StatusKind::Success, "Status updated.".into())),
                        Err(e) => self.flash = Some((StatusKind::Error, e.to_string())),
                    };
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn view(&self) -> Result<Html> {
        let mut flash = NO_FLASH;
        if let Some((kind, msg)) = &self.flash {
            match kind {
                StatusKind::Success => flash.success = msg,
                _ => flash.error = msg,
            }
        }
        let mut body = Html::EMPTY;
        let mut detail = Html::EMPTY;
        let mut history = Html::EMPTY;
        if let Some(record) = &self.record {
            let mut facts = vec![
                Fact {
                    title: "Status",
                    id: "status",
                    value: Html::text(&record.status),
                },
                Fact {
                    title: "Code",
                    id: "code",
                    value: Html::text(&record.code),
                },
            ];
            let fields = if self.kind == "item" {
                vec![
                    ("name", "Name"),
                    ("unit", "Unit"),
                    ("default_price", "Default price"),
                    ("tax_code", "Tax code"),
                    ("tax_rate", "Tax rate (%)"),
                ]
            } else {
                vec![
                    ("customer_code", "Customer code"),
                    ("currency_code", "Currency"),
                    ("reference", "Reference"),
                    ("valid_until", "Valid until"),
                    ("subtotal", "Subtotal"),
                    ("tax_total", "Tax"),
                    ("total", "Total"),
                ]
            };
            for (key, label) in fields {
                let value = if self.kind == "item" {
                    let mut f = InlineField::new(
                        format!("item-{key}"),
                        key,
                        label,
                        text(&record.facts, key),
                    )
                    .version(record.revision)
                    .status(self.commits.get(key));
                    f.save_event = "update_item".into();
                    f.editable = self.manage;
                    render(&InlineEdit { field: f })?
                } else {
                    Html::text(text(&record.facts, key))
                };
                facts.push(Fact {
                    title: label,
                    id: key,
                    value,
                });
            }
            detail = render(&Facts {
                id: "record-facts",
                items: facts,
            })?;
            if let Some(lines) = record.facts["lines"].as_array() {
                let columns = vec![
                    ("description", "Description"),
                    ("quantity", "Quantity"),
                    ("unit", "Unit"),
                    ("unit_price", "Unit price"),
                    ("tax_code", "Tax code"),
                    ("tax_total", "Tax"),
                    ("total", "Total"),
                ];
                let mut table = FlexTable::new(
                    "document-lines",
                    columns
                        .iter()
                        .map(|(k, l)| FlexColumn::new(k, k, l))
                        .collect(),
                    lines
                        .iter()
                        .enumerate()
                        .map(|(i, l)| {
                            let mut row = FlexRow::new(i.to_string());
                            for (k, _) in &columns {
                                row = row.slot(k, Html::text(text(l, k)));
                            }
                            row
                        })
                        .collect(),
                );
                table.caption = "Lines";
                body = render(&table)?;
            }
            if !self.history.is_empty() {
                history = render(&History {
                    rows: &self.history,
                })?;
            }
        } else {
            let rows = self
                .rows
                .iter()
                .map(|r| {
                    let mut row = FlexRow::new(r.id.to_string()).slot(
                        "code",
                        render(&RecordLink {
                            workspace: self.framed,
                            kind: crate::MODULE,
                            record_id: &r.id.to_string(),
                            href: &format!("{}/{}", path(self.kind), r.id),
                            class: "",
                            body: Html::text(&r.code),
                        })?,
                    );
                    for (key, value) in [
                        ("name", text(&r.facts, "name")),
                        ("unit", text(&r.facts, "unit")),
                        ("price", text(&r.facts, "default_price")),
                        ("tax", text(&r.facts, "tax_code")),
                        ("customer", text(&r.facts, "customer_code")),
                        ("status", &r.status),
                    ] {
                        row = row.slot(key, Html::text(value));
                    }
                    row = row.slot(
                        "total",
                        Html::text(&format!(
                            "{} {}",
                            text(&r.facts, "total"),
                            text(&r.facts, "currency_code")
                        )),
                    );
                    Ok(row)
                })
                .collect::<Result<Vec<_>>>()?;
            let mut table = FlexTable::new("commerce-table", self.columns.flex_columns(), rows);
            table.event = "grid";
            table.zoom = self.columns.zoom;
            table.suggestions = self.columns.flex_suggestions();
            table.add_query = &self.columns.add_query;
            table.caption = title(self.kind);
            table.framed = false;
            if self.rows.is_empty() {
                table.empty = Some(TableEmpty::State(EmptyState::missing(
                    "No records here",
                    "Create a record or change the filters.",
                )));
            }
            body = render(&table)?;
        }
        let controls = vec![
            ToolbarControl::search(
                "commerce-search",
                "search",
                "Search",
                &self.list.search,
                "Code or customer code",
            ),
            ToolbarControl::select(
                "commerce-status",
                "status",
                "Status",
                self.list.filters.get("status").map_or("", String::as_str),
                [
                    "",
                    "draft",
                    "sent",
                    "accepted",
                    "rejected",
                    "expired",
                    "confirmed",
                    "cancelled",
                ]
                .iter()
                .map(|s| Choice::new(if s.is_empty() { "All" } else { *s }, *s))
                .collect(),
            ),
        ];
        let toolbar = render(&FilterToolbar::new("commerce-filters", controls))?;
        let pagination = render(&Pagination::new(
            "commerce-pages",
            PageCounts::new(self.list.page, self.list.page_size, self.total),
            &[25, 50, 100],
        ))?;
        let action_options = if self.kind == "quotation" {
            match self.record.as_ref().map(|r| r.status.as_str()) {
                Some("draft") => vec!["sent"],
                Some("sent") => vec!["accepted", "rejected", "expired"],
                _ => vec![],
            }
        } else {
            match self.record.as_ref().map(|r| r.status.as_str()) {
                Some("draft") => {
                    if self.confirm {
                        vec!["confirmed", "cancelled"]
                    } else {
                        vec!["cancelled"]
                    }
                }
                Some("confirmed") => vec!["cancelled"],
                _ => vec![],
            }
        };
        let action_fields = vec![
            Field::in_form("form", "status")
                .label("New status")
                .kind(InputKind::Select)
                .options(action_options.iter().map(|s| Choice::new(*s, *s)).collect()),
            Field::in_form("form", "reason")
                .label("Reason")
                .hint("Required when rejecting a quotation or cancelling an order."),
        ];
        let confirmation = if let Some((_, _, input)) = &self.pending {
            render(&ConfirmDialog {
                id: "commerce-confirm",
                consequence: "Change this record?",
                detail: &format!(
                    "Set status to {}. {}",
                    text(input, "status"),
                    if text(input, "status") == "confirmed" {
                        "Finance can use this confirmed order to prepare an invoice."
                    } else {
                        "The change stays in the status history."
                    }
                ),
                confirm: "Confirm",
                working: "Saving…",
                confirm_event: "confirm_action",
                cancel_event: "close",
            })?
        } else {
            Html::EMPTY
        };
        render(&View {
            variant: if self.id.is_some() {
                PageVariant::Detail
            } else {
                PageVariant::List
            },
            create_title: match self.kind {
                "item" => "New product",
                "quotation" => "New quotation",
                _ => "New sales order",
            },
            create_event: match self.kind {
                "item" => "create_item",
                "quotation" => "create_quotation",
                _ => "create_order",
            },
            status_event: if self.kind == "quotation" {
                "quotation_transition"
            } else {
                "order_transition"
            },
            heading: self.heading(),
            kind: self.kind,
            record_id: self.id.map(|id| id.to_string()).unwrap_or_default(),
            back: path(self.kind),
            is_detail: self.id.is_some(),
            manage: self.manage,
            editing: self.editing,
            flash,
            fields: self.fields(),
            lines: self.line_fields(),
            line_count: self.line_count,
            body,
            detail,
            history,
            toolbar,
            pagination,
            action_fields,
            actions: self.manage && !action_options.is_empty(),
            confirmation,
            errors: self.errors.values().flatten().cloned().collect(),
        })
    }
}
impl LiveState<Scope, Error> for State {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match &r.input {
                LiveInput::Params(q) => self.list = list(q)?,
                LiveInput::Event(v) => self.event(&r, v).await?,
                _ => {}
            }
            self.load(r.scope).await?;
            Ok(LiveBody {
                html: self.view()?.to_string(),
                query: self.list.to_params(),
                redirect: self.redirect.take(),
                rows: self.rows.len(),
                ..LiveBody::default()
            })
        })
    }
}
#[derive(Template)]
#[template(path = "sales.html")]
struct View<'a> {
    variant: PageVariant,
    create_title: &'a str,
    create_event: &'a str,
    status_event: &'a str,
    heading: String,
    kind: &'a str,
    record_id: String,
    back: &'a str,
    is_detail: bool,
    manage: bool,
    editing: bool,
    flash: FlashMap<'a>,
    fields: Vec<Field>,
    lines: Vec<Vec<Field>>,
    line_count: usize,
    body: Html,
    detail: Html,
    history: Html,
    toolbar: Html,
    pagination: Html,
    action_fields: Vec<Field>,
    actions: bool,
    confirmation: Html,
    errors: Vec<String>,
}
#[derive(Template)]
#[template(path = "history.html")]
struct History<'a> {
    rows: &'a [Value],
}

fn posted(v: &Value) -> HashMap<String, String> {
    v.as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| {
            let key = k.strip_prefix("form[")?.strip_suffix(']')?;
            let value = v.as_str()?;
            Some((key.to_owned(), value.chars().take(1024).collect()))
        })
        .take(810)
        .collect()
}
