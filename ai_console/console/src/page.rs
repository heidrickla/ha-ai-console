//! The page, composed once from its three embedded sources into one document.

const HTML: &str = include_str!("web/index.html");
const CSS: &str = include_str!("web/style.css");
const JS: &str = include_str!("web/app.js");

pub fn page() -> String {
    HTML.replace("/*STYLE*/", CSS).replace("/*SCRIPT*/", JS)
}
