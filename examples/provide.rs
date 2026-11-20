use std::time::Duration;

use dfui::widgets::{Snapshot, Styled, future_builder, provide, text};
use dfui::{Element, Scope, sleep};

#[derive(Clone)]
struct Greeter {
    name: &'static str,
}

impl Greeter {
    async fn greet(self) -> String {
        sleep(Duration::from_millis(800)).await;
        format!("Hello from {}!", self.name)
    }
}

fn view() -> Element {
    let greeting = future_builder(
        (),
        |scope: Scope| scope.get::<Greeter>().expect("provided").clone().greet(),
        |snap: &Snapshot<Option<String>>| {
            text(snap.data.as_deref().unwrap_or("Asking the provider…")).size(20.0)
        },
    );
    provide(
        Greeter {
            name: "the provider",
        },
        greeting,
    )
    .padding(16.0)
    .into()
}

fn main() {
    dfui::run(view);
}
