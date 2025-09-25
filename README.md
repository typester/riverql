# RiverQL

RiverQL exposes [River window manager](https://isaacfreund.com/software/river/) state over GraphQL.
It ships a server that bridges River's Wayland status protocol into GraphQL queries and
subscriptions, plus a CLI client for driving `graphql-transport-ws` streams.

## Features

- GraphQL access to River output/seat state (tags, layouts, focused view, mode)
- Real-time subscriptions via `graphql-transport-ws`
- Lightweight CLI client for ad-hoc GraphQL subscriptions

## Getting Started

Most setups launch the server inside River's init script:

```bash
riverql --server &
```

By default this creates a Unix socket under `$XDG_RUNTIME_DIR/riverql.sock`. To
override, use `--listen`, e.g. `riverql --server --listen tcp://127.0.0.1:8080`.

The server logs via `tracing`; tune with `RUST_LOG` (for instance
`RUST_LOG=riverql=debug`).

### GraphQL Endpoints

- HTTP/WS endpoint: `/graphql`
- GraphiQL UI: `/graphiql`
- Schema SDL: `/schema`

Example query:

```graphql
{
  outputs {
    outputId
    name
    focusedTags
    viewTags
    urgentTags
    layoutName
  }
  seatFocusedOutput { outputId name }
}
```

Subscription example:

```graphql
subscription {
  riverEvents {
    __typename
    ... on OutputFocusedTags { outputId name tags }
    ... on SeatFocusedOutput { outputId name }
  }
}
```

### WebSocket Client Mode

### Installing

```bash
cargo install --path .
```

This installs a `riverql` binary in your Cargo bin directory.

### Client mode

When a widget or script (for example an eww widget) needs data, invoke `riverql`
without `--server`:

```bash
riverql 'subscription { riverEvents { __typename } }'
```

Key points:

- Inline queries or `@file.graphql`
- Reads stdin when no query argument is supplied
- Uses the default endpoint derived from `--listen`; override with
  `--endpoint` if needed (supports both `unix://path#/graphql` and
  `ws://host:port/path` formats)

### Using with [eww](https://elkowar.github.io/eww/)

Add the server to your River init script (`riverql --server &`). Then, inside
`eww.yuck`, you can consume RiverQL in two ways:

Polling a query:

```clojure
(defpoll river_outputs :interval "5s"
  "riverql 'query { outputs { outputId name focusedTags } }' | jq -c '.data.outputs'")

(defwidget river-tags []
  (box :orientation "vertical"
    (for output in river_outputs
      (box :class "tag-row"
        (label :text (format "%s" (. output 'name)))
        (label :text (format "%s" (. output 'focusedTags)))))))
```

Listening for live events:

```clojure
(deflisten river_events :initial "{}"
  "riverql 'subscription { riverEvents { __typename ... on OutputFocusedTags { outputId name tags } } }' | jq -c '.data.riverEvents'")

(defwidget river-event-feed []
  (box :orientation "vertical"
    (label :text (format "Latest event: %s" river_events))))
```

`defpoll` is ideal for periodic snapshots (e.g. populating a list of outputs),
while `deflisten` reacts instantly to subscription pushes. Both examples assume
`riverql` is on `PATH` and that `jq` is available to compact JSON.

## License

Code in this repository is licensed under MIT; see [LICENSE](LICENSE).

The XML files under `protocol/` are copied from upstream River (GPL-3.0-or-later)
and wlroots (MIT). They retain their original licensing. Consult the upstream
projects for full details.
