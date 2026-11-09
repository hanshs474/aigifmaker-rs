# aigifmaker

Turn text into animated GIFs with AI GIF Maker. Native GIF generation — no video conversion. Type what you want and download your GIF in about a minute.

This is the official Rust SDK for [AI GIF Maker](https://aigifmaker.ai/?utm_source=crates&utm_medium=package): send a text prompt, get back the URL of a real animated GIF, ready to drop into a chat, a README or your own app.

```rust
let url = aigifmaker::generate("a cat dancing in the rain, pixel art")?;
println!("{url}");
```

One call submits the prompt, waits for the GIF and returns its URL.

## Install

```bash
cargo add aigifmaker
```

Blocking HTTP via `ureq`; the only other dependency is `serde_json`.

## API key

Sign in, create a key at https://aigifmaker.ai/settings/apikeys, then export it:

```bash
export AIGIFMAKER_API_KEY=sk-...
```

Each GIF costs 1 credit on your AI GIF Maker account, the same as on the website. New accounts get one free GIF (it carries a small aigifmaker.ai badge). Failed generations are refunded. Credits are on the [pricing](https://aigifmaker.ai/pricing?utm_source=crates&utm_medium=package) page.

## A client of your own

```rust
use std::time::Duration;

let mut client = aigifmaker::Client::new("sk-...");   // or Client::from_env()
client.timeout = Duration::from_secs(600);           // default: 5 minutes, polling every 3 seconds

let left = client.credits()?;                         // free to call
let id = client.submit("a rocket launching, retro cartoon")?;
let status = client.status(&id)?;                     // pending | processing | success | failed
let url = client.wait(&id)?;                          // polls until the GIF is ready
```

On an account that has not bought credits yet, a GIF can wait in a queue first: `status.queued` is `true` and `status.queue_eta` gives the seconds left. `wait` and `generate` keep polling through it.

## Errors

Every call returns `aigifmaker::Error`, split by what to do next:

- `Error::Auth`: no key, or the key is wrong or deleted
- `Error::Quota`: the account is out of credits
- `Error::Rejected`: moderation refused the prompt, or the account is cooling down after a refusal; reword rather than retry
- `Error::Failed`: the GIF was accepted and then failed; the credit comes back
- `Error::Timeout`: the wait ran out; the GIF may still finish in your history on the site
- `Error::Api`: anything else, with the service's message verbatim

## Examples

```bash
AIGIFMAKER_API_KEY=sk-... cargo run --example generate -- "a cat dancing in the rain, pixel art"
AIGIFMAKER_API_KEY=sk-... cargo run --example credits
```

## Support

support@aigifmaker.ai

MIT licensed.
