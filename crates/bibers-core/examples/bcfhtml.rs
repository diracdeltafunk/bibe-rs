//! `bcfhtml IN.bcf OUT.html [IN OUT]…`: render `.bcf` files as
//! `--convert-control` does (for comparison with `xsltproc bcf.xsl`).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    for pair in args.chunks(2) {
        let [inp, out] = pair else {
            eprintln!("usage: bcfhtml IN OUT [IN OUT]...");
            std::process::exit(2);
        };
        let bytes = std::fs::read(inp).expect("read");
        let text = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => e.into_bytes().iter().map(|&b| b as char).collect(),
        };
        match roxmltree::Document::parse_with_options(&text, relaxng::parsing_options()) {
            Ok(doc) => std::fs::write(out, bibers_core::bcfhtml::render(&doc)).expect("write"),
            Err(e) => eprintln!("{inp}: {e}"),
        }
    }
}
