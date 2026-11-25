use bilibi_module_registry::{Contribution, Provenance};
use bilibi_templates::{
    PrintContract, PrintDefault, PrintFacts, PrintField, PrintKind, PrintValue, PRINT_CATALOGUE,
};
use jiff::civil::date;
use rust_decimal::Decimal;

pub(crate) const KEY: &str = "commerce.sales.delivery_order";

const fn field(
    id: &'static str,
    labels: &'static [(&'static str, &'static str)],
    kind: PrintKind,
    sample: PrintValue<'static>,
) -> PrintField {
    PrintField {
        id,
        labels,
        kind,
        optional: false,
        required_output: true,
        sample,
    }
}

const fn text(
    id: &'static str,
    labels: &'static [(&'static str, &'static str)],
    sample: &'static str,
) -> PrintField {
    field(id, labels, PrintKind::Text, PrintValue::Text(sample))
}

const COLUMNS: &[PrintField] = &[
    field(
        "order_line",
        &[("en", "Order line"), ("ms", "Baris pesanan")],
        PrintKind::Integer,
        PrintValue::Integer(2),
    ),
    text("item", &[("en", "Item"), ("ms", "Item")], "Sample material"),
    field(
        "quantity",
        &[("en", "Quantity"), ("ms", "Kuantiti")],
        PrintKind::Decimal,
        PrintValue::Decimal(Decimal::ONE),
    ),
    field(
        "ordered",
        &[("en", "Ordered"), ("ms", "Dipesan")],
        PrintKind::Decimal,
        PrintValue::Decimal(Decimal::from_parts(2, 0, 0, false, 0)),
    ),
    PrintField {
        optional: true,
        ..text(
            "product",
            &[("en", "Product code"), ("ms", "Kod produk")],
            "EXAMPLE-14",
        )
    },
    text("unit", &[("en", "Unit"), ("ms", "Unit")], "box"),
];

const ROW: PrintFacts<'static> = &[
    ("order_line", COLUMNS[0].sample),
    ("item", COLUMNS[1].sample),
    ("quantity", COLUMNS[2].sample),
    ("ordered", COLUMNS[3].sample),
    ("product", COLUMNS[4].sample),
    ("unit", COLUMNS[5].sample),
];

pub(crate) static CONTRACT: PrintContract = PrintContract {
    key: KEY,
    label: "Delivery order",
    version: "1",
    states: &["draft", "shipped", "delivered", "cancelled"],
    fields: &[
        text(
            "number",
            &[("en", "Delivery order"), ("ms", "Pesanan penghantaran")],
            "DO-27",
        ),
        text(
            "issuer",
            &[("en", "From"), ("ms", "Daripada")],
            "Example Works",
        ),
        text(
            "recipient",
            &[("en", "Customer"), ("ms", "Pelanggan")],
            "Example Trading",
        ),
        text(
            "ship_to",
            &[("en", "Ship to"), ("ms", "Alamat penghantaran")],
            "Unit 14\nExample Trade Centre\nSelangor",
        ),
        text(
            "order",
            &[("en", "Sales order"), ("ms", "Pesanan jualan")],
            "SO-42",
        ),
        text("status", &[("en", "Status"), ("ms", "Status")], "Shipped"),
        PrintField {
            optional: true,
            ..field(
                "ship_date",
                &[
                    ("en", "Dispatch date (UTC)"),
                    ("ms", "Tarikh penghantaran (UTC)"),
                ],
                PrintKind::Date,
                PrintValue::Date(date(2026, 10, 15)),
            )
        },
        PrintField {
            optional: true,
            required_output: false,
            ..text(
                "reference",
                &[("en", "Reference"), ("ms", "Rujukan")],
                "Site 01 delivery",
            )
        },
        field(
            "lines",
            &[("en", "Items"), ("ms", "Butiran")],
            PrintKind::Rows(COLUMNS),
            PrintValue::Rows(&[ROW]),
        ),
    ],
    base_language: "en",
    defaults: &[
        PrintDefault {
            language: "en",
            version: "1",
            layout: include_str!("../prints/delivery.en.json"),
        },
        PrintDefault {
            language: "ms",
            version: "1",
            layout: include_str!("../prints/delivery.ms.json"),
        },
    ],
};

inventory::submit! { Contribution::new(PRINT_CATALOGUE, KEY, Provenance {
    module: crate::MODULE, source: file!(), line: line!(),
}, &CONTRACT) }
