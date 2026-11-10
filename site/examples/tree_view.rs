use dfui::Element;
use dfui::model::{Keyed, Tree};
use dfui::widgets::{Styled, column, text, tree_view};

#[derive(Clone)]
struct Entry(u64, &'static str);

impl Keyed for Entry {
    fn key(&self) -> u64 {
        self.0
    }
}

fn files() -> Tree<Entry> {
    let mut t = Tree::new();
    let src = t.push(None, Entry(1, "src"));
    t.push(Some(src), Entry(2, "lib.rs"));
    let widgets = t.push(Some(src), Entry(3, "widgets"));
    t.push(Some(widgets), Entry(4, "button.rs"));
    t.push(Some(widgets), Entry(5, "list.rs"));
    t.push(None, Entry(6, "README.md"));
    t
}

fn view() -> Element {
    let tree = tree_view(files(), |e: &Entry, _| text(e.1));
    column([tree.fill().into()]).fill().padding(16.0).into()
}

fn main() {
    dfui::run(view);
}
