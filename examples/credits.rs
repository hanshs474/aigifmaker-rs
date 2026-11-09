// AIGIFMAKER_API_KEY=sk-... cargo run --example credits
fn main() {
    match aigifmaker::Client::from_env().credits() {
        Ok(n) => println!("{n}"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1)
        }
    }
}
