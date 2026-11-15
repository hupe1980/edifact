+++
title = "Async Integration"
description = "Patterns for using the synchronous edifact parser from tokio without blocking the runtime."
weight = 100
+++

`edifact` is synchronous: its readers take `std::io::Read`. Parsing is CPU work
over a buffer, so an async parser would add a runtime dependency without
removing any blocking. Read with your runtime, parse synchronously. The
patterns below use Tokio and carry over to other runtimes.

```bash
cargo add edifact
cargo add tokio --features rt-multi-thread,macros,fs,io-util,sync
```

## Choosing a pattern

| Payload | Source | Pattern |
|---|---|---|
| Small | Any | A — buffer, then `from_bytes` |
| Large | File | B — `spawn_blocking` + a reader |
| Large, processed as it arrives | Any | C — channel bridge |
| Large, typed | Any | D — typed channel bridge |
| Any | `AsyncRead` (`TcpStream`, HTTP body) | E — bounded buffer, then A or B |

The [default limits](@/docs/parsing.md) apply on every path: 64 KiB per
segment, 16 MiB per message, 256 MiB in total.

## A — Read into memory, parse

```rust,no_run
use edifact::from_bytes;

async fn process_edi_file(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = tokio::fs::read(path).await?;
    for seg in from_bytes(&bytes) {
        println!("{}", seg?.tag);
    }
    Ok(())
}
```

Parsing runs on the async worker and is linear in the input. Move large
payloads to `spawn_blocking` (B).

## B — `spawn_blocking`

```rust,no_run
use edifact::{OwnedMessageWindow, message_windows_from_reader};

async fn process_large_interchange(path: String)
    -> Result<Vec<OwnedMessageWindow>, Box<dyn std::error::Error>>
{
    let windows = tokio::task::spawn_blocking(move || {
        let f = std::fs::File::open(&path)?;
        message_windows_from_reader(f).collect::<Result<Vec<_>, _>>()
    })
    .await??; // JoinError, then EdifactError
    Ok(windows)
}
```

## C — Channel bridge

Each `UNH`…`UNT` window reaches the async side as soon as it is parsed. The
channel capacity is the backpressure: a slow consumer blocks the producer.

```rust,no_run
use edifact::{EdifactError, OwnedMessageWindow, message_windows_from_reader};
use tokio::sync::mpsc;

async fn stream_interchange(bytes: Vec<u8>) -> Result<usize, EdifactError> {
    let (tx, mut rx) = mpsc::channel::<Result<OwnedMessageWindow, EdifactError>>(8);

    tokio::task::spawn_blocking(move || {
        for window in message_windows_from_reader(std::io::Cursor::new(bytes)) {
            if tx.blocking_send(window).is_err() {
                break; // consumer dropped: stop parsing
            }
        }
    });

    let mut messages = 0;
    while let Some(window) = rx.recv().await {
        let window = window?;
        messages += 1;
        let _ = window; // process, then drop
    }
    Ok(messages)
}
```

## D — Typed channel bridge

Pattern C with `deserialize_messages_from_reader`:

```rust,no_run
use edifact::{EdifactError, EdifactDeserialize, deserialize_messages_from_reader};
use tokio::sync::mpsc;

# #[derive(Debug, EdifactDeserialize)]
# #[edifact(segment = "BGM")]
# struct Bgm { #[edifact(element = 0)] doc_code: String }
# #[derive(Debug, EdifactDeserialize)]
# struct OrderMessage { bgm: Option<Bgm> }
async fn stream_typed_messages(bytes: Vec<u8>) -> Result<Vec<OrderMessage>, EdifactError> {
    let (tx, mut rx) = mpsc::channel::<Result<OrderMessage, EdifactError>>(8);

    tokio::task::spawn_blocking(move || {
        let cursor = std::io::Cursor::new(bytes);
        for msg in deserialize_messages_from_reader::<OrderMessage, _>(cursor) {
            if tx.blocking_send(msg).is_err() {
                break;
            }
        }
    });

    let mut messages = Vec::new();
    while let Some(result) = rx.recv().await {
        messages.push(result?);
    }
    Ok(messages)
}
```

## E — From `AsyncRead`

Buffer the stream, capped so a peer cannot exhaust memory, then parse as in A
or B:

```rust,no_run
use tokio::io::AsyncReadExt;

async fn parse_from_async_reader<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
) -> Result<usize, Box<dyn std::error::Error>> {
    let mut buf = Vec::new();
    reader.take(256 << 20).read_to_end(&mut buf).await?;
    Ok(edifact::from_bytes(&buf).collect::<Result<Vec<_>, _>>()?.len())
}
```

## Errors

`spawn_blocking` returns a `JoinError` if the closure panics; `?` propagates it:

```rust,no_run
async fn count(bytes: Vec<u8>) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let n = tokio::task::spawn_blocking(move || edifact::from_bytes(&bytes).count()).await?;
    Ok(n)
}
```

## Example

```bash
cargo run -p edifact --example cookbook_async_tokio_integration
```

[`cookbook_async_tokio_integration.rs`](https://github.com/hupe1980/edifact/tree/main/crates/edifact/examples/cookbook_async_tokio_integration.rs)
runs patterns A, B and C.

## Next steps

- [Streaming](@/docs/streaming.md) — synchronous streaming APIs
- [Performance](@/docs/performance.md) — memory budgets and benchmarking
