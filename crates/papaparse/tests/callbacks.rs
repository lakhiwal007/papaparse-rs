//! Hand ports of the upstream cases that pass JS functions in the config, which the
//! JSON-driven conformance harness has to skip. Each test name matches the upstream
//! description.

#![allow(clippy::unwrap_used)]

use papaparse::{DynamicTyping, Field, ParseConfig, Parser, Row, Value, parse};

fn fields<'a>(row: &Row<'a>) -> Vec<Value<'a>> {
    match row {
        Row::Fields(f) => f.clone(),
        Row::Record(_) => panic!("expected an array row"),
    }
}

fn record<'r, 'a>(row: &'r Row<'a>) -> Vec<(&'r str, &'r Value<'a>)> {
    match row {
        Row::Record(r) => r.iter().collect(),
        Row::Fields(_) => panic!("expected a record row"),
    }
}

fn s(v: &str) -> Value<'_> {
    Value::from(v)
}

#[test]
fn duplicated_header_names_with_header_transform() {
    let cfg = ParseConfig::default()
        .header(true)
        .transform_header(|h, _| h.to_lowercase());
    let out = Parser::new(&cfg).unwrap().parse("A,A,A,A\n1,2,3,4");
    assert_eq!(
        out.data,
        vec![vec!["a", "a_1", "a_2", "a_3"], vec!["1", "2", "3", "4"]]
    );
    let renamed: Vec<_> = out.meta.renamed_headers.unwrap().into_iter().collect();
    assert_eq!(
        renamed,
        [("a_1", "a"), ("a_2", "a"), ("a_3", "a")].map(|(k, v)| (k.to_owned(), v.to_owned()))
    );
    assert_eq!(out.meta.cursor, 15);
}

#[test]
fn header_rows_are_transformed_when_transform_header_function_is_provided() {
    let cfg = ParseConfig::default()
        .header(true)
        .transform_header(|h, _| h.to_lowercase());
    let out = parse("A,B,C\r\na,b,c", &cfg).unwrap();
    assert_eq!(out.errors, []);
    assert_eq!(
        record(&out.data[0]),
        [("a", &s("a")), ("b", &s("b")), ("c", &s("c"))]
    );
}

#[test]
fn transform_header_accepts_an_optional_index_attribute() {
    let cfg = ParseConfig::default()
        .header(true)
        .transform_header(|h, i| {
            if i % 2 == 1 {
                h.to_lowercase()
            } else {
                h.to_owned()
            }
        });
    let out = parse("A,B,C\r\na,b,c", &cfg).unwrap();
    assert_eq!(
        record(&out.data[0]),
        [("A", &s("a")), ("b", &s("b")), ("C", &s("c"))]
    );
}

#[test]
fn callback_delimiter() {
    let cfg = ParseConfig::default().delimiter_fn(|input| format!("{} ", &input[1..2]));
    let out = parse("a$ b$ c", &cfg).unwrap();
    assert_eq!(out.errors, []);
    assert_eq!(fields(&out.data[0]), [s("a"), s("b"), s("c")]);
}

#[test]
fn dynamic_typing_by_indices_can_be_determined_by_function() {
    let typing = DynamicTyping::Dynamic(std::sync::Arc::new(
        |f| matches!(f, Field::Index(i) if i % 2 == 0),
    ));
    let out = parse(
        "001,002,003",
        &ParseConfig::default().dynamic_typing_with(typing),
    )
    .unwrap();
    assert_eq!(
        fields(&out.data[0]),
        [Value::Number(1.0), s("002"), Value::Number(3.0)]
    );
}

#[test]
fn dynamic_typing_by_headers_can_be_determined_by_function() {
    let typing = DynamicTyping::Dynamic(std::sync::Arc::new(
        |f| matches!(f, Field::Name(n) if n.ends_with("_as_int")),
    ));
    let cfg = ParseConfig::default()
        .header(true)
        .dynamic_typing_with(typing);
    let out = parse("A_as_int,B,C_as_int\r\n001,002,003", &cfg).unwrap();
    assert_eq!(
        record(&out.data[0]),
        [
            ("A_as_int", &Value::Number(1.0)),
            ("B", &s("002")),
            ("C_as_int", &Value::Number(3.0))
        ]
    );
}

#[test]
fn custom_transform_function_is_applied_to_values() {
    let cfg = ParseConfig::default().transform(|v, _| v.to_lowercase());
    let out = parse("A,B,C\r\nd,e,f", &cfg).unwrap();
    assert_eq!(fields(&out.data[0]), [s("a"), s("b"), s("c")]);
    assert_eq!(fields(&out.data[1]), [s("d"), s("e"), s("f")]);
}

#[test]
fn custom_transform_accepts_column_number_also() {
    let cfg = ParseConfig::default().transform(|v, f| match f {
        Field::Index(i) if i % 2 == 1 => v.to_lowercase(),
        _ => v.to_owned(),
    });
    let out = parse("A,B,C\r\nd,e,f", &cfg).unwrap();
    assert_eq!(fields(&out.data[0]), [s("A"), s("b"), s("C")]);
    assert_eq!(fields(&out.data[1]), [s("d"), s("e"), s("f")]);
}

#[test]
fn custom_transform_accepts_header_name_when_using_header() {
    let cfg = ParseConfig::default().header(true).transform(|v, f| {
        if f == Field::Name("B") {
            v.to_uppercase()
        } else {
            v.to_owned()
        }
    });
    let out = parse("A,B,C\r\nd,e,f", &cfg).unwrap();
    assert_eq!(
        record(&out.data[0]),
        [("A", &s("d")), ("B", &s("E")), ("C", &s("f"))]
    );
}
