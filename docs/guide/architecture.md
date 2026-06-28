# Architecture

`minecraft-mcp-rs` is layered **types → logic → state → bot → mcp → ui**, all
in one crate.

```
┌──────────────────────────────────────────────────┐
│                   egui Desktop UI               │
│  ┌──────────┐  ┌──────────┐                     │
│  │  Status  │  │ Settings │                     │
│  └────┬─────┘  └────┬─────┘                     │
│       └──────┬───────┘                          │
│              │ reads/writes                      │
│              ▼                                   │
│       ┌──────────┐                               │
│       │SharedState│  (ArcSwap + RwLock + Atomics)│
│       └────┬─────┘                               │
└────────────┼─────────────────────────────────────┘
             │ reads
┌────────────┼─────────────────────────────────────┐
│   MCP Server (rmcp, stdio or HTTP transport)    │
│  ┌──────────┐   ┌───────────────────────────┐   │
│  │  Router  │──▶│ tools_query/movement/... │   │
│  └────┬─────┘   └───────────────────────────┘   │
│       │ sends BotCommand (tokio mpsc + oneshot)  │
│       ▼                                           │
│  ┌──────────┐                                    │
│  │ BotEngine│ (azalea client + bevy_ecs)         │
│  └──────────┘                                    │
└──────────────────────────────────────────────────┘
```

## Threading Model

The bot runs on a **background OS thread** with its own tokio runtime. The egui
desktop UI runs on the **main thread**. The bot connection is spawned on a
dedicated OS thread (not `tokio::spawn`) because azalea's
`ClientBuilder::start` internally creates a `LocalSet` which is `!Send`,
preventing it from running on a multi-threaded runtime.

The two sides communicate through two mechanisms:

- **`Arc<SharedState>`** — the thread-safe hub. World snapshots are stored
  behind `ArcSwap` for **lock-free reads**, so the UI and MCP tools never
  block the bot. Configuration uses `RwLock`; online/connecting/disconnect
  flags are `AtomicBool`; container handles and chat messages use `Mutex`
  (all with poison-recovery via `unwrap_or_else(|e| e.into_inner())`).
- **`BotCommand` channel** — a tokio `mpsc` channel for commands plus a
  `oneshot` channel for responses. The command receiver is held in a
  `ReceiverSlot` and leased out via a `ReceiverLease` on `Event::Spawn`; when
  the executor is aborted on `Event::Disconnect`, the lease drops and returns
  the receiver to the slot for the next reconnect.

`Event::Disconnect` also writes `AppExit::Success` to the ECS
(`bot.ecs.lock().write_message(AppExit::Success)`) so `ClientBuilder::start`
returns and the connect loop can retry (azalea 0.15.1 removed `Client::exit()`).
A `CancellationToken` is cancelled on disconnect so the reconnect backoff
sleep returns immediately instead of blocking shutdown.
