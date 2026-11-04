use crate::{contributions::PAGES, prepare::text, Record, Sales, CONFIRM, MANAGE, VIEW};
use askama::Template;
use bilibi_agent_api::page::{Call, PageContracts};
use bilibi_database::{Error, Result};
use bilibi_datetime::{DisplayContext, DisplayPolicy, Format, Precision};
use bilibi_grid::{PageColumns, PageViews};
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
    views: Arc<PageViews>,
    display: Arc<DisplayPolicy>,
    locales: Arc<bilibi_locale::Locales>,
}
impl Pages {
    pub fn new(
        sales: Arc<Sales>,
        contracts: Arc<PageContracts>,
        views: Arc<PageViews>,
        display: Arc<DisplayPolicy>,
        locales: Arc<bilibi_locale::Locales>,
    ) -> Self {
        Self {
            sales,
            contracts,
            views,
            display,
            locales,
        }
    }
}
fn title(k: &str) -> &'static str {
    match k {
        "item" => "Products",
        "quotation" => "Quotations",
        "delivery" => "Delivery orders",
        _ => "Sales orders",
    }
}
fn path(k: &str) -> &'static str {
    match k {
        "item" => "/commerce/items",
        "quotation" => "/commerce/quotations",
        "delivery" => "/commerce/delivery-orders",
        _ => "/commerce/sales-orders",
    }
}
fn kind(op: &str) -> &'static str {
    match op {
        "item" | "items" => "item",
        "quotation" | "quotations" => "quotation",
        "delivery" | "deliveries" => "delivery",
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
                    "shipped",
                    "delivered",
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
    columns: PageColumns,
    display_policy: Arc<DisplayPolicy>,
    locales: Arc<bilibi_locale::Locales>,
    display: DisplayContext,
    column_params: HashMap<String, String>,
    columns_open: bool,
    rows: Vec<Record>,
    record: Option<Record>,
    source: Option<Record>,
    history: Vec<Value>,
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
        } else if k == "delivery" {
            vec![
                ("code", "Number"),
                ("customer", "Customer"),
                ("order", "Sales order"),
                ("status", "Status"),
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
            columns: PageColumns::mount(
                p.views.clone(),
                &format!("commerce-{k}"),
                cols.into_iter()
                    .map(|(k, l)| BuiltinColumn::new(k, l))
                    .collect(),
            )
            .and_then(|columns| {
                columns.with_record_slots(if k == "item" {
                    item_fields::ALL
                } else {
                    document_fields::ALL
                })
            })
            .map_err(|_| Error::Internal)?,
            display_policy: p.display.clone(),
            locales: p.locales.clone(),
            display: DisplayContext::anonymous(),
            column_params: q.clone(),
            columns_open: false,
            rows: vec![],
            record: None,
            source: None,
            history: vec![],
            manage: false,
            confirm: false,
            editing: k == "delivery" && q.contains_key("order_number"),
            form: q
                .get("order_number")
                .map(|n| HashMap::from([("order_number".into(), n.clone())]))
                .unwrap_or_default(),
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
                        "delivery" => "Delivery order",
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
        let locale = self.locales.resolve(&**c, s, None).await?.locale;
        self.display = self.display_policy.display(&**c, s, None, locale).await?;
        drop(c);
        if !self.columns_open {
            self.columns.from_params(s, &self.column_params).await?;
            self.columns_open = true;
        }
        if let Some(id) = self.id {
            self.record = Some(self.sales.get(s, company, self.kind, id).await?);
            if self.kind != "item" {
                self.history = self.sales.history(s, company, id).await?;
            }
        } else {
            let status = self.list.filters.get("status").cloned().unwrap_or_default();
            self.rows = self
                .sales
                .list(
                    s,
                    company,
                    self.kind,
                    &self.list.search,
                    &status,
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
                &status,
            )
            .await? as u64;
            drop(c);
            if self
                .list
                .clamp_to_last_page(self.total.div_ceil(self.list.page_size), true)
            {
                self.rows = self
                    .sales
                    .list(
                        s,
                        company,
                        self.kind,
                        &self.list.search,
                        &status,
                        self.list.page_size as i64,
                        self.list.offset().min(i64::MAX as u64) as i64,
                    )
                    .await?;
            }
            self.columns
                .load(
                    s,
                    &self.rows.iter().map(|r| json!(r.id)).collect::<Vec<_>>(),
                    &self.display,
                )
                .await?;
        }
        if self.kind == "delivery" && self.editing && self.source.is_none() {
            if let Some(number) = self
                .form
                .get("order_number")
                .and_then(|v| v.parse::<i64>().ok())
            {
                let c = self.sales.db.get().await?;
                let row=c.query_opt("SELECT id FROM commerce_sales_documents WHERE tenant_id=$1 AND company_id=$2 AND kind='order' AND number=$3 AND status='confirmed'", &[&s.tenant(),&company,&number]).await?;
                drop(c);
                if let Some(row) = row {
                    let record = self.sales.get(s, company, "order", row.get(0)).await?;
                    let lines = record.facts["lines"].as_array().ok_or(Error::Internal)?;
                    self.line_count = lines.len();
                    for (i, line) in lines.iter().enumerate() {
                        self.form.insert(
                            format!("line_{i}_quantity"),
                            text(line, "available").to_owned(),
                        );
                    }
                    self.source = Some(record);
                } else {
                    self.flash =
                        Some((StatusKind::Error, "Choose a confirmed sales order.".into()));
                }
            }
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
        if self.kind == "delivery" {
            return vec![
                self.field("order_number", "Sales order number")
                    .hint("Load the lines to choose full or partial quantities."),
                self.field("reference", "Reference"),
            ];
        }
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
            fields.push(self.field("quotation_number","Accepted quotation number").hint("Leave blank to enter an order directly. A quotation supplies the customer and lines, so leave them empty below."));
        }
        fields
    }
    fn line_fields(&self) -> Vec<Vec<Field>> {
        if self.kind == "delivery" {
            let Some(lines) = self
                .source
                .as_ref()
                .and_then(|r| r.facts["lines"].as_array())
            else {
                return Vec::new();
            };
            return lines.iter().enumerate().map(|(i,line)| {
                let label=format!("{} ({})",text(line,"description"),text(line,"unit"));
                let hint=format!("Ordered: {}. Delivered: {}. Remaining: {}. Available: {}. Leave blank or enter 0 to skip this line.",text(line,"quantity"),text(line,"delivered"),text(line,"remaining"),text(line,"available"));
                vec![self.field(&format!("line_{i}_quantity"),&label).hint(hint)]
            }).collect();
        }
        (0..self.line_count)
            .map(|i| {
                let mut fields = vec![self
                    .field(&format!("line_{i}_item_code"), "Product code")
                    .hint("Leave blank to enter a typed line.")];
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
        if self.kind == "delivery" {
            v["order_number"] = json!(self
                .form
                .get("order_number")
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(0));
            let lines = (0..self.line_count)
                .filter_map(|i| {
                    let quantity = self.form.get(&format!("line_{i}_quantity"))?.trim();
                    if quantity.is_empty()
                        || quantity
                            .parse::<rust_decimal::Decimal>()
                            .is_ok_and(|q| q.is_zero())
                    {
                        return None;
                    }
                    Some(json!({"order_line_number":i+1,"quantity":quantity}))
                })
                .collect::<Vec<_>>();
            v["lines"] = json!(lines);
            return v;
        }
        if self.kind == "order" {
            if let Some(q) = self.form.get("quotation_number").filter(|s| !s.is_empty()) {
                v["quotation_number"] = json!(q.parse::<i64>().unwrap_or(0));
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
                    if let Some(code) = self
                        .form
                        .get(&format!("line_{i}_item_code"))
                        .filter(|s| !s.is_empty())
                    {
                        line["item_code"] = json!(code);
                    }
                    if text(&line, "description").is_empty()
                        && line["item_code"].is_null()
                        && text(&line, "quantity").is_empty()
                    {
                        None
                    } else {
                        Some(line)
                    }
                })
                .collect::<Vec<_>>();
            if !lines.is_empty() || v["quotation_number"].is_null() {
                v["lines"] = json!(lines);
            }
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
                self.columns
                    .handle(r.scope, &params::string_values(v))
                    .await?;
            }
            "clear-flash" => self.flash = None,
            "new" => {
                self.editing = true;
                self.form.clear();
                self.errors.clear();
                self.line_count = 1;
                self.source = None;
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
            "create_item" | "create_quotation" | "create_order" | "create_delivery" => {
                self.form = posted(v);
                if self.kind == "delivery" && text(v, "intent") == "load_order" {
                    self.source = None;
                    return Ok(());
                }
                if self.kind == "delivery"
                    && self.source.as_ref().is_none_or(|r| {
                        self.form.get("order_number").map(String::as_str) != Some(r.code.as_str())
                    })
                {
                    self.source = None;
                    self.flash = Some((
                        StatusKind::Error,
                        "Load the sales order lines before saving.".into(),
                    ));
                    return Ok(());
                }
                if text(v, "intent") == "add_line" {
                    if self.line_count < 100 {
                        self.line_count += 1;
                    }
                    return Ok(());
                }
                let key = match self.kind {
                    "item" => "commerce.sales.item.create",
                    "quotation" => "commerce.sales.quotation.create",
                    "delivery" => "commerce.sales.delivery.create",
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
            "quotation_transition" | "order_transition" | "delivery_transition" => {
                if let Some(record) = &self.record {
                    let status = text(v, "form[status]");
                    let reason = text(v, "form[reason]");
                    let input = json!({"company_id":r.scope.current_company().map_or(0,|c|c.get()),"id":record.id,"revision":record.revision,"status":status,"reason":reason});
                    let key = if self.kind == "quotation" {
                        "commerce.sales.quotation.transition"
                    } else if self.kind == "delivery" {
                        "commerce.sales.delivery.transition"
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
                            self.flash = Some((StatusKind::Error, review_refusal(&review)))
                        }
                        Err(e) => self.flash = Some((StatusKind::Error, e.to_string())),
                    };
                }
            }
            "confirm_action" => {
                if let Some((_event, key, input)) = self.pending.take() {
                    let key = if key == "commerce.sales.quotation.transition" {
                        "commerce.sales.quotation.transition"
                    } else if key == "commerce.sales.delivery.transition" {
                        "commerce.sales.delivery.transition"
                    } else {
                        "commerce.sales.order.transition"
                    };
                    let call = Call {
                        scope: r.scope,
                        credential: r.credential,
                        service: PAGES,
                        event: "confirm_action",
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
            } else if self.kind == "delivery" {
                vec![
                    ("customer_code", "Customer code"),
                    ("order_number", "Sales order number"),
                    ("reference", "Reference"),
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
            for (key, _) in fields {
                let label = if self.kind == "item" {
                    item_fields::get(key).label
                } else {
                    document_fields::get(key).label
                };
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
                let columns = if self.kind == "delivery" {
                    vec![
                        ("description", "Description"),
                        ("ordered_quantity", "Ordered"),
                        ("quantity", "Delivery quantity"),
                        ("unit", "Unit"),
                    ]
                } else if self.kind == "order" {
                    vec![
                        ("description", "Description"),
                        ("quantity", "Ordered"),
                        ("delivered", "Delivered"),
                        ("remaining", "Remaining"),
                        ("available", "Available"),
                        ("unit", "Unit"),
                        ("unit_price", "Unit price"),
                        ("total", "Total"),
                    ]
                } else {
                    vec![
                        ("description", "Description"),
                        ("quantity", "Quantity"),
                        ("unit", "Unit"),
                        ("unit_price", "Unit price"),
                        ("tax_code", "Tax code"),
                        ("tax_total", "Tax"),
                        ("total", "Total"),
                    ]
                };
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
                    rows: self
                        .history
                        .iter()
                        .map(|h| {
                            let at = text(h, "at")
                                .parse::<jiff::Timestamp>()
                                .map(|at| {
                                    self.display
                                        .render(at, Format::DateTime, Precision::Second)
                                        .text
                                })
                                .unwrap_or_default();
                            HistoryRow {
                                at,
                                from: text(h, "from").into(),
                                to: text(h, "to").into(),
                                user: h["user_id"].to_string(),
                                impersonator: h["impersonator_id"]
                                    .as_i64()
                                    .map(|i| i.to_string())
                                    .unwrap_or_default(),
                                reason: text(h, "reason").into(),
                            }
                        })
                        .collect(),
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
                        ("order", text(&r.facts, "order_number")),
                        ("name", text(&r.facts, "name")),
                        ("unit", text(&r.facts, "unit")),
                        ("price", text(&r.facts, "default_price")),
                        ("tax", text(&r.facts, "tax_code")),
                        ("tax_rate", text(&r.facts, "tax_rate")),
                        ("currency_code", text(&r.facts, "currency_code")),
                        ("reference", text(&r.facts, "reference")),
                        ("valid_until", text(&r.facts, "valid_until")),
                        ("subtotal", text(&r.facts, "subtotal")),
                        ("tax_total", text(&r.facts, "tax_total")),
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
            let mut table = self.columns.flex_table("commerce-table", rows);
            table.event = "grid";

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
        let mut controls = vec![ToolbarControl::search(
            "commerce-search",
            "search",
            "Search",
            &self.list.search,
            "Code or customer code",
        )];
        if self.kind != "item" {
            let statuses: &[&str] = if self.kind == "quotation" {
                &["", "draft", "sent", "accepted", "rejected", "expired"]
            } else if self.kind == "delivery" {
                &["", "draft", "shipped", "delivered", "cancelled"]
            } else {
                &["", "draft", "confirmed", "cancelled"]
            };
            controls.push(ToolbarControl::select(
                "commerce-status",
                "status",
                "Status",
                self.list.filters.get("status").map_or("", String::as_str),
                statuses
                    .iter()
                    .map(|s| Choice::new(if s.is_empty() { "All" } else { *s }, *s))
                    .collect(),
            ));
        }
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
        } else if self.kind == "delivery" {
            match self.record.as_ref().map(|r| r.status.as_str()) {
                Some("draft") => vec!["shipped", "cancelled"],
                Some("shipped") => vec!["delivered", "cancelled"],
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
                .hint("Required when rejecting a quotation or cancelling an order or delivery."),
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
                "delivery" => "New delivery order",
                _ => "New sales order",
            },
            create_event: match self.kind {
                "item" => "create_item",
                "quotation" => "create_quotation",
                "delivery" => "create_delivery",
                _ => "create_order",
            },
            status_event: if self.kind == "quotation" {
                "quotation_transition"
            } else if self.kind == "delivery" {
                "delivery_transition"
            } else {
                "order_transition"
            },
            heading: self.heading(),
            kind: self.kind,
            delivery_link: self
                .record
                .as_ref()
                .filter(|r| r.kind == "order" && r.status == "confirmed")
                .map(|r| format!("/commerce/delivery-orders?order_number={}", r.code))
                .unwrap_or_default(),
            source_loaded: self.source.is_some(),
            record_id: self.id.map(|id| id.to_string()).unwrap_or_default(),
            back: path(self.kind),
            is_detail: self.id.is_some(),
            manage: self.manage,
            editing: self.editing,
            flash,
            fields: self.fields(),
            lines: self.line_fields(),
            line_description: if self.kind == "delivery" {
                "Choose the quantity for this delivery."
            } else {
                "Enter a product code and quantity, or enter all line details. Product lines use the catalog price, unit and tax."
            },
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
                LiveInput::Params(q) => {
                    self.list = list(q)?;
                    self.column_params = (*q).clone();
                    self.columns_open = false;
                }
                LiveInput::Event(v) => self.event(&r, v).await?,
                _ => {}
            }
            self.load(r.scope).await?;
            Ok(LiveBody {
                html: self.view()?.to_string(),
                query: {
                    let mut q = self.list.to_params();
                    q.extend(self.columns.params());
                    q
                },
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
    delivery_link: String,
    source_loaded: bool,
    back: &'a str,
    is_detail: bool,
    manage: bool,
    editing: bool,
    flash: FlashMap<'a>,
    fields: Vec<Field>,
    lines: Vec<Vec<Field>>,
    line_count: usize,
    line_description: &'a str,
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
struct History {
    rows: Vec<HistoryRow>,
}
struct HistoryRow {
    at: String,
    from: String,
    to: String,
    user: String,
    impersonator: String,
    reason: String,
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

bilibi_ui::record_fields! { mod item_fields {
 CODE:bilibi_ui::RecordField::new("code","Code"),NAME:bilibi_ui::RecordField::new("name","Name"),UNIT:bilibi_ui::RecordField::new("unit","Unit"),
 PRICE:bilibi_ui::RecordField::new("default_price","Default price").slot("price"),TAX:bilibi_ui::RecordField::new("tax_code","Tax code").slot("tax"),RATE:bilibi_ui::RecordField::new("tax_rate","Tax rate (%)"),STATUS:bilibi_ui::RecordField::new("status","Status")
}}
bilibi_ui::record_fields! { mod document_fields {
 ORDER:bilibi_ui::RecordField::new("order_number","Sales order number"),CODE:bilibi_ui::RecordField::new("code","Number"),CUSTOMER:bilibi_ui::RecordField::new("customer_code","Customer code").slot("customer"),CURRENCY:bilibi_ui::RecordField::new("currency_code","Currency"),REFERENCE:bilibi_ui::RecordField::new("reference","Reference"),EXPIRY:bilibi_ui::RecordField::new("valid_until","Valid until"),NET:bilibi_ui::RecordField::new("subtotal","Subtotal"),TAX:bilibi_ui::RecordField::new("tax_total","Tax"),TOTAL:bilibi_ui::RecordField::new("total","Total"),STATUS:bilibi_ui::RecordField::new("status","Status")
}}

fn review_refusal(review: &Value) -> String {
    let messages = review["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["error"]["fields"].as_object())
        .flat_map(|fields| fields.values())
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if messages.is_empty() {
        "This change is no longer available. Reload the record and try again.".into()
    } else {
        messages.join(" ")
    }
}
