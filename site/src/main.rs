//! `dfui-site OUT_DIR` writes the docs pages into OUT_DIR: the Markdown in
//! `pages/`, the architecture doc, and one page per example in `examples/`
//! (its `//!` header as text, then its live frame and code). `scripts/site.sh`
//! adds each example's wasm under `wasm/`.

use std::fs;
use std::path::Path;

use pulldown_cmark::{Options, Parser, html};

/// Widget pages, in navigation order; each is `examples/<name>.rs`.
const WIDGETS: &[&str] = &[
    "text",
    "button",
    "layout",
    "scroll",
    "stateful",
    "text_input",
    "toggle",
    "slider",
    "radio",
    "segmented",
    "tabs",
    "provide",
    "future_builder",
    "stream_builder",
];

struct Page {
    /// Output path relative to the site root.
    path: String,
    title: String,
    body: String,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [out] = args.as_slice() else {
        eprintln!("usage: dfui-site OUT_DIR");
        std::process::exit(2);
    };
    let site = Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |p: &Path| fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));

    let docs = [
        ("index.html", site.join("pages/index.md")),
        (
            "getting-started.html",
            site.join("pages/getting-started.md"),
        ),
        ("concepts.html", site.join("pages/concepts.md")),
        ("architecture.html", site.join("../docs/architecture.md")),
    ];
    let mut pages: Vec<Page> = docs
        .iter()
        .map(|(path, md)| {
            let src = read(md);
            Page {
                path: path.to_string(),
                title: title(&src),
                body: markdown(&src),
            }
        })
        .collect();
    for name in WIDGETS {
        let src = read(&site.join(format!("examples/{name}.rs")));
        let (doc, code) = split(&src);
        let title = title(&doc);
        let body = format!(
            "{}<div class=\"frame\"><iframe src=\"../run.html?example={name}\" \
             title=\"{} live example\" loading=\"lazy\"></iframe></div>\n\
             <pre><code class=\"language-rust\">{}</code></pre>\n\
             <p class=\"hint\">Run it natively: <code>cargo run -p dfui-site --example {name}</code></p>\n",
            markdown(&doc),
            escape(&title),
            escape(code),
        );
        pages.push(Page {
            path: format!("widgets/{name}.html"),
            title,
            body,
        });
    }

    let out = Path::new(out);
    fs::create_dir_all(out.join("widgets")).expect("create output dirs");
    for page in &pages {
        fs::write(out.join(&page.path), layout(page, &pages)).expect("write page");
    }
    fs::write(out.join("run.html"), RUN_PAGE).expect("write run.html");
    println!("wrote {} pages to {}", pages.len(), out.display());
}

/// The first `# ` heading.
fn title(md: &str) -> String {
    md.lines()
        .find_map(|l| l.strip_prefix("# "))
        .unwrap_or("dfui")
        .trim()
        .to_string()
}

/// Splits an example into its `//!` header (as Markdown) and the code after it.
fn split(src: &str) -> (String, &str) {
    let mut doc = String::new();
    let mut rest = src;
    while let Some(line) = rest.lines().next() {
        let Some(text) = line.strip_prefix("//!") else {
            break;
        };
        doc.push_str(text.strip_prefix(' ').unwrap_or(text));
        doc.push('\n');
        rest = rest[line.len()..].strip_prefix('\n').unwrap_or("");
    }
    (doc, rest.trim_start_matches('\n'))
}

fn markdown(md: &str) -> String {
    let mut out = String::new();
    html::push_html(&mut out, Parser::new_ext(md, Options::ENABLE_TABLES));
    out
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn layout(page: &Page, pages: &[Page]) -> String {
    let root = "../".repeat(page.path.matches('/').count());
    let link = |p: &Page| {
        let current = if p.path == page.path {
            " aria-current=\"page\""
        } else {
            ""
        };
        format!(
            "<li><a href=\"{root}{}\"{current}>{}</a></li>",
            p.path,
            escape(&p.title)
        )
    };
    let (guides, widgets) = pages.split_at(pages.len() - WIDGETS.len());
    let guides: String = guides.iter().map(link).collect();
    let widgets: String = widgets.iter().map(link).collect();
    LAYOUT
        .replace("{{title}}", &escape(&page.title))
        .replace("{{root}}", &root)
        .replace("{{guides}}", &guides)
        .replace("{{widgets}}", &widgets)
        .replace("{{body}}", &page.body)
}

const LAYOUT: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{{title}} · dfui</title>
<style>
:root {
  --bg: #f7f7f9; --panel: #ffffff; --ink: #1d2030; --muted: #5d6273;
  --line: #e1e3ea; --accent: #3b6fe0; --code-bg: #eef0f4;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #14161d; --panel: #1c1f28; --ink: #e6e8ef; --muted: #9aa0b2;
    --line: #2c303c; --accent: #7aa2ff; --code-bg: #262a36;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0; background: var(--bg); color: var(--ink);
  font: 16px/1.65 system-ui, -apple-system, "Segoe UI", sans-serif;
  display: grid; grid-template-columns: 230px minmax(0, 1fr);
}
nav {
  position: sticky; top: 0; height: 100vh; overflow-y: auto;
  padding: 28px 18px; border-right: 1px solid var(--line);
}
nav .brand { display: block; font-weight: 700; font-size: 22px; margin: 0 0 18px 10px; color: var(--ink); text-decoration: none; }
nav h2 { font-size: 12px; letter-spacing: .06em; text-transform: uppercase; color: var(--muted); margin: 22px 10px 6px; }
nav ul { list-style: none; margin: 0; padding: 0; }
nav a {
  display: block; padding: 4px 10px; border-radius: 6px;
  color: var(--muted); text-decoration: none; font-size: 15px;
}
nav a:hover { color: var(--ink); background: var(--panel); }
nav a[aria-current] { color: var(--accent); background: var(--panel); font-weight: 600; }
main { max-width: 860px; padding: 40px 48px 96px; }
main h1 { font-size: 34px; line-height: 1.2; margin: 0 0 16px; }
main h2 { font-size: 23px; margin: 36px 0 10px; }
main h3 { font-size: 18px; margin: 28px 0 8px; }
main a { color: var(--accent); }
p, ul, ol { margin: 0 0 14px; }
code {
  font: 14px/1.5 ui-monospace, "SF Mono", Menlo, Consolas, monospace;
  background: var(--code-bg); padding: 1px 5px; border-radius: 4px;
}
pre {
  margin: 0 0 16px; padding: 16px 18px; overflow-x: auto;
  background: var(--panel); border: 1px solid var(--line); border-radius: 12px;
}
pre code { background: none; padding: 0; font-size: 13.5px; }
table { border-collapse: collapse; margin: 0 0 16px; font-size: 15px; display: block; overflow-x: auto; }
th, td { border: 1px solid var(--line); padding: 6px 10px; text-align: left; vertical-align: top; }
th { background: var(--panel); }
.frame {
  margin: 20px 0 14px; border: 1px solid var(--line); border-radius: 12px;
  overflow: hidden; background: #f2f3f7; aspect-ratio: 16 / 10;
}
.frame iframe { width: 100%; height: 100%; border: 0; display: block; }
.hint { color: var(--muted); font-size: 14px; }
@media (max-width: 760px) {
  body { display: block; }
  nav { position: static; height: auto; border-right: 0; border-bottom: 1px solid var(--line); padding: 16px; }
  nav ul { display: flex; flex-wrap: wrap; gap: 2px; }
  main { padding: 24px 16px 64px; }
}
</style>
</head>
<body>
<nav>
<a class="brand" href="{{root}}index.html">dfui</a>
<h2>Guide</h2><ul>{{guides}}</ul>
<h2>Widgets</h2><ul>{{widgets}}</ul>
</nav>
<main>
{{body}}
</main>
</body>
</html>
"#;

/// Hosts one example: loads `wasm/<name>.js`, whose `main` takes over the page.
const RUN_PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>dfui example</title>
<style>
html, body { margin: 0; height: 100%; overflow: hidden; background: #f2f3f7; }
canvas { display: block; width: 100%; height: 100%; outline: none; touch-action: none; }
p { margin: 0; padding: 24px; font: 15px/1.5 system-ui, sans-serif; color: #5d6273; }
code { font-size: 13px; background: #e4e6ec; padding: 1px 4px; border-radius: 4px; }
</style>
</head>
<body>
<script type="module">
const name = new URLSearchParams(location.search).get("example") ?? "";
const known = /^[a-z_]+$/.test(name);
const gpu = navigator.gpu && await navigator.gpu.requestAdapter().catch(() => null);
if (known && gpu) {
  const wasm = await import(`./wasm/${name}.js`);
  await wasm.default();
} else {
  const p = document.createElement("p");
  p.innerHTML = known
    ? "This live example needs WebGPU. On Linux it is often off by default: " +
      "enable <code>chrome://flags/#enable-unsafe-webgpu</code> in Chrome, or " +
      "<code>dom.webgpu.enabled</code> in Firefox's <code>about:config</code>, then reload."
    : "Unknown example.";
  document.body.append(p);
}
</script>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples_split_into_docs_and_code() {
        let (doc, code) = split("//! # Title\n//!\n//! Text `x`.\n\nuse a;\nfn main() {}\n");
        assert_eq!(doc, "# Title\n\nText `x`.\n");
        assert_eq!(code, "use a;\nfn main() {}\n");
        assert_eq!(title(&doc), "Title");
    }

    #[test]
    fn every_widget_has_an_example_with_a_title() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        for name in WIDGETS {
            let src = fs::read_to_string(dir.join(format!("{name}.rs"))).unwrap();
            let (doc, code) = split(&src);
            assert_ne!(title(&doc), "dfui", "{name} has no `//! # Title`");
            assert!(code.contains("dfui_site::run(view)"), "{name}");
        }
        let files = fs::read_dir(&dir).unwrap().count();
        assert_eq!(files, WIDGETS.len(), "every example is listed in WIDGETS");
    }
}
