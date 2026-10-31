//! # Provide
//!
//! `provide(value, child)` makes `value` available to everything below it.
//! Builders read the nearest one with `scope.get::<T>()` when they start their
//! source, so external state (clients, channels, stores) is handed down
//! without passing it through every function.
//!
//! Here a provided `Greeter` service answers after a short delay.

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
    dfui_site::run(view);
}
