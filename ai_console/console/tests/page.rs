mod common;
use ai_console::page::page;
use common::*;

#[test]
fn the_page_is_one_document_with_its_style_and_script_inline() {
    let p = page();
    assert!(p.starts_with("<!doctype html>"));
    assert!(p.contains("<style>") && p.contains("<script>"));
    assert!(
        !p.contains("/*STYLE*/") && !p.contains("/*SCRIPT*/"),
        "placeholders replaced"
    );
    for tab in ["usage", "plans", "routes", "providers"] {
        assert!(p.contains(&format!("data-tab=\"{tab}\"")), "tab {tab}");
    }
    assert!(p.contains("data-theme=\"dark\""), "dark by default");
}

#[test]
fn the_page_reaches_nothing_outside_itself() {
    // The SVG namespace is an identifier, not a request.
    let p = page().replace("http://www.w3.org/2000/svg", "");
    assert!(
        !p.contains("https://") && !p.contains("http://"),
        "no external URL"
    );
    for bad in ["src=\"/", "href=\"/", "fetch('/", "fetch(\"/", "url(/"] {
        assert!(!p.contains(bad), "absolute URL {bad}");
    }
    assert!(p.contains("api/feeds"), "relative feed URL");
}

#[test]
fn get_root_serves_the_page_and_unknown_paths_404() {
    let r = rig("page");
    let reply = get(&r, "/");
    assert_eq!(reply.status, 200);
    assert!(reply.content_type.starts_with("text/html"));
    assert_eq!(text(&reply), page());
    assert_eq!(get(&r, "/index.html").status, 404);
    assert_eq!(get(&r, "/healthz").status, 200);
    assert_eq!(r.app.handle("GET", "/?ingress=1", None, b"").status, 200);
}
