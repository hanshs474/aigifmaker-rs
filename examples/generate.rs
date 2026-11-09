// AIGIFMAKER_API_KEY=sk-... cargo run --example generate -- "a cat dancing in the rain, pixel art"
fn main() {
    let prompt = std::env::args().nth(1).unwrap_or_else(|| "a cat dancing in the rain, pixel art".into());
    match aigifmaker::generate(&prompt) {
        Ok(url) => println!("{url}"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1)
        }
    }
}
