use bilibi_module_registry::{Contribution, Provenance};
use bilibi_templates::{
    PrintContract, PrintDefault, PrintFacts, PrintField, PrintKind, PrintValue, PRINT_CATALOGUE,
};
use rust_decimal::Decimal;

const fn amount(cents: u32) -> PrintValue<'static> {
    PrintValue::Money {
        amount: Decimal::from_parts(cents, 0, 0, false, 2),
        currency: "MYR",
    }
}

macro_rules! field {
    ($id:literal, $en:literal, $ms:literal, $kind:ident, $sample:expr) => {
        PrintField {
            id: $id,
            labels: &[("en", $en), ("ms", $ms)],
            kind: PrintKind::$kind,
            sample: $sample,
            optional: false,
            required_output: true,
        }
    };
}

const fn optional(mut value: PrintField) -> PrintField {
    value.optional = true;
    value
}
const COLUMNS: &[PrintField] = &[
    field!(
        "item",
        "Item",
        "Butiran",
        Text,
        PrintValue::Text("Example product")
    ),
    field!(
        "quantity",
        "Quantity",
        "Kuantiti",
        Text,
        PrintValue::Text("2 EA")
    ),
    field!(
        "price",
        "List unit price",
        "Harga senarai seunit",
        Money,
        amount(10_000)
    ),
    field!(
        "discount",
        "Unit discount",
        "Diskaun seunit",
        Money,
        amount(1_000)
    ),
    field!("net", "Before tax", "Sebelum cukai", Money, amount(18_000)),
    field!(
        "tax_code",
        "Tax code and rate",
        "Kod dan kadar cukai",
        Text,
        PrintValue::Text("SV6 6%")
    ),
    field!("tax", "Tax", "Cukai", Money, amount(1_080)),
    field!("total", "Total", "Jumlah", Money, amount(19_080)),
];
const ROW: PrintFacts<'static> = &[
    ("item", COLUMNS[0].sample),
    ("quantity", COLUMNS[1].sample),
    ("price", COLUMNS[2].sample),
    ("discount", COLUMNS[3].sample),
    ("net", COLUMNS[4].sample),
    ("tax_code", COLUMNS[5].sample),
    ("tax", COLUMNS[6].sample),
    ("total", COLUMNS[7].sample),
];

macro_rules! contract {
    ($name:ident, $key:literal, $label:literal, $file:literal, $states:expr, $date:expr, $extra:expr) => {
        pub(crate) static $name: PrintContract = PrintContract {
            key: $key,
            label: $label,
            version: "1",
            states: $states,
            fields: &[
                field!(
                    "number",
                    "Document number",
                    "Nombor dokumen",
                    Text,
                    PrintValue::Text("00042")
                ),
                field!(
                    "state",
                    "Status",
                    "Status",
                    Text,
                    PrintValue::Text("Confirmed")
                ),
                $date,
                $extra,
                optional(field!(
                    "issuer",
                    "Issued by",
                    "Pengeluar",
                    Text,
                    PrintValue::Text("Example Works Sdn. Bhd.")
                )),
                field!(
                    "customer",
                    "Customer",
                    "Pelanggan",
                    Text,
                    PrintValue::Text("Example Trade")
                ),
                optional(field!(
                    "address",
                    "Billing address",
                    "Alamat bil",
                    Text,
                    PrintValue::Text("Example Road, Petaling Jaya")
                )),
                optional(field!(
                    "reference",
                    "Reference",
                    "Rujukan",
                    Text,
                    PrintValue::Text("Request 42")
                )),
                PrintField {
                    id: "lines",
                    labels: &[],
                    kind: PrintKind::Rows(COLUMNS),
                    sample: PrintValue::Rows(&[ROW]),
                    optional: false,
                    required_output: true,
                },
                field!(
                    "subtotal",
                    "Subtotal before tax",
                    "Jumlah kecil sebelum cukai",
                    Money,
                    amount(18_000)
                ),
                field!(
                    "tax_total",
                    "Total tax",
                    "Jumlah cukai",
                    Money,
                    amount(1_080)
                ),
                field!(
                    "total",
                    "Total including tax",
                    "Jumlah termasuk cukai",
                    Money,
                    amount(19_080)
                ),
            ],
            base_language: "en",
            defaults: &[
                PrintDefault {
                    language: "en",
                    version: "1",
                    layout: include_str!(concat!("../prints/", $file, ".en.json")),
                },
                PrintDefault {
                    language: "ms",
                    version: "1",
                    layout: include_str!(concat!("../prints/", $file, ".ms.json")),
                },
            ],
        };
        inventory::submit! {
            Contribution::new(PRINT_CATALOGUE, $key, Provenance {
                module: crate::MODULE, source: file!(), line: line!()
            }, &$name)
        }
    };
}

contract!(
    QUOTATION,
    "commerce.sales.quotation",
    "Quotation",
    "quotation",
    &["sent", "accepted", "rejected", "expired"],
    optional(field!(
        "price_on",
        "Pricing date",
        "Tarikh harga",
        Date,
        PrintValue::Date(jiff::civil::date(2026, 10, 1))
    )),
    field!(
        "valid_until",
        "Valid through",
        "Sah sehingga",
        Date,
        PrintValue::Date(jiff::civil::date(2026, 10, 31))
    )
);
contract!(
    ORDER,
    "commerce.sales.order_confirmation",
    "Order confirmation",
    "order-confirmation",
    &["confirmed", "cancelled"],
    field!(
        "confirmed_on",
        "Confirmation date (UTC)",
        "Tarikh pengesahan (UTC)",
        Date,
        PrintValue::Date(jiff::civil::date(2026, 10, 1))
    ),
    optional(field!(
        "quotation",
        "Source quotation",
        "Sebut harga asal",
        Text,
        PrintValue::Text("00042")
    ))
);
