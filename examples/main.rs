use evtly_rs::*;

fn main() {
    let mut eb = EventBus::default();

    eb.register(
        Box::new(|evt: &str| {
            println!("{evt}");
            EventPropagation::Propagate
        }),
        u32::MAX,
    );

    dbg!(eb.post("Hiii"));
}
