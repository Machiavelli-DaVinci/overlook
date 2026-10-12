//! Prints the sanitized HTML and extracted text for an HTML file: `cargo run -p overlook-core --example sanitize_dump -- file.html`
use overlook_core::sanitize::{sanitize_html, RemoteContent};
fn main() {
    let path = std::env::args().nth(1).expect("path to an .html file");
    let html = std::fs::read_to_string(path).expect("read file");
    let s = sanitize_html(&html, RemoteContent::Blocked);
    println!("--- html ---\n{}\n--- text ---\n{}\n--- blocked remote: {}", s.html, s.text, s.blocked_remote);
}
