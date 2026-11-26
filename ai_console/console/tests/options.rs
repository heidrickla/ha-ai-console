use ai_console::options::parse;

#[test]
fn a_missing_options_file_accepts_no_push_and_pulls_no_router() {
    let o = parse(None, "").unwrap();
    assert!(o.push_sources.is_empty());
    assert!(o.router_url.is_empty());
    let o = parse(Some(r#"{"push_sources":[],"router_url":""}"#), "").unwrap();
    assert!(o.push_sources.is_empty() && o.router_url.is_empty());
}

#[test]
fn listed_sources_and_a_trailing_slash_are_normalized() {
    let o = parse(
        Some(r#"{"push_sources":["10.0.0.1","::1"],"router_url":"http://r:1/"}"#),
        "127.0.0.1, ",
    )
    .unwrap();
    assert_eq!(o.push_sources.len(), 3);
    assert_eq!(o.router_url, "http://r:1");
}

#[test]
fn an_entry_that_is_not_an_address_refuses_the_start_and_names_it() {
    for bad in ["1.2.3", "192.0.2.10/24", "workstation", "999.1.1.1"] {
        let e = parse(Some(&format!(r#"{{"push_sources":["{bad}"]}}"#)), "").unwrap_err();
        assert!(e.contains(bad) && e.contains("push_sources"), "{e}");
    }
    assert!(parse(None, "nope").unwrap_err().contains("nope"));
}

#[test]
fn damaged_or_wrongly_typed_options_refuse_the_start() {
    assert!(parse(Some("{broken"), "").unwrap_err().contains("not JSON"));
    assert!(
        parse(Some(r#"{"push_sources":"10.0.0.1"}"#), "")
            .unwrap_err()
            .contains("push_sources")
    );
    assert!(
        parse(Some(r#"{"router_url":"ftp://x"}"#), "")
            .unwrap_err()
            .contains("router_url")
    );
    assert!(
        parse(Some(r#"{"router_url":7}"#), "")
            .unwrap_err()
            .contains("router_url")
    );
}

#[test]
fn anything_but_an_object_refuses_the_start() {
    for bad in ["[]", "null", "7", "\"x\"", "true", ""] {
        let e = parse(Some(bad), "").unwrap_err();
        assert!(e.contains("options.json"), "{bad}: {e}");
    }
}
