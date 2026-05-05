You are an expert [0.7 Dioxus](https://dioxuslabs.com/learn/0.7) assistant. Dioxus 0.7 changes every api in dioxus. Only use this up to date documentation. `cx`, `Scope`, and `use_state` are gone

Provide concise code examples with detailed descriptions

# Project workflow

Two build pipelines run side-by-side:

- **Rust (Dioxus)**: `dx serve --package sharam-ui` from the repo root.
- **Tailwind v4**: source is `tailwind.css`; compiled output is `assets/tailwind.css` (the file `asset!()` references). Run `npm run watch:css` (or `npm run build:css` once) inside `sharam-ui/`. The compiled file is what the browser loads — do not edit it by hand.

`tailwind.css` uses `@source "./src/**/*.rs";` so any Tailwind class typed in a `.rs` file is picked up.

# Auth

Google sign-in uses Google Identity Services (loaded via `Dioxus.toml` → `web.resource.script`). The browser reads the client ID at runtime from `<meta name="sharam-google-client-id">`, which is populated from the `GOOGLE_CLIENT_ID` const in `src/main.rs`. That const is filled at build time by `build.rs`, which:

1. Prefers env var `SHARAM_GOOGLE__CLIENT_ID` (matches the `figment` env-var convention used elsewhere).
2. Otherwise reads `[google].client_id` from `../Sharam.toml`.
3. Falls back to `"MISSING_GOOGLE_CLIENT_ID"` and emits a cargo warning if neither is found.

Cargo reruns `build.rs` when `Sharam.toml` or the env var changes, so editing the toml triggers a rebuild on next `dx serve`. **Only the client ID is baked into the WASM** — `client_secret` and `redirect_uri` stay server-side. After `POST /api/auth/google` succeeds, `pages/login.rs` stores the ID token in `localStorage["sharam_id_token"]` and redirects to `/dashboard`. Subsequent API calls send it as `Authorization: Bearer …`. There is no session cookie; the gateway re-verifies the JWT on every protected request.

# Pages

Each page lives in `src/pages/` and the route table is defined in `src/main.rs`. They share the `Sidenav` shell from `pages/sidenav.rs`. Quick map of what each page does and which gateway endpoint(s) it talks to:

| Page (file) | Route | Endpoints used |
|---|---|---|
| `login.rs` | `/login` | `POST /api/auth/google` |
| `dashboard.rs` | `/dashboard` | `GET /api/me/ventures` |
| `create_tenant.rs` | `/ventures/new` | `POST /api/tenants` |
| `venture.rs` | `/ventures/:slug` | `GET /api/me/ventures`, `GET\|PATCH /api/tenants/:slug/settings`, `GET /api/tenants/:slug/members`, `GET /api/tenants/:slug/contributions/me`, `POST /api/tenants/:slug/contributions` |
| `admin.rs` | `/ventures/:slug/invites` | `GET\|POST /api/tenants/:slug/invites`, `DELETE …/:key`, `POST …/revoke` |
| `profile.rs` | `/profile` | (decodes JWT locally for display) |
| `settings.rs` | `/settings` | placeholder |

## Venture page — `MyPeriodPanel`

`venture.rs` hosts the contribution UX. The `MyPeriodPanel` component is the part to know:

- **Loads** caller's period roll-up via `fetch_my_contributions(slug)` → `(PeriodSummary, Vec<Contribution>)`. The summary carries `dues_cents`, `paid_cents`, `remaining_cents`, plus the resolved `period`/`cadence`/`currency`.
- **Renders** a progress bar (positive when settled, amber when partial, neutral when empty), three small facts (Dues / Paid / Remaining via `fmt_money`), a payment-history list, and either a submit form or a "Settled" banner when `remaining_cents == 0`.
- **Submits** via `submit_contribution(slug, { amount_cents, note? })`. Major-unit input is parsed as `f64` and converted to cents with `(major * 100.0).round() as i64`. The gateway derives the period from settings — never send `period` from the client.
- **On 422** the gateway returns `dues_cap_exceeded: …` or `period_locked: …`; both come back as `ApiError::Other(msg)` and are rendered inline under the form. After a successful POST the panel calls `data.restart()` so the bar and history refresh without a full page reload.

# HTTP — `crate::api`

**All HTTP calls go through `src/api.rs`. Do not call `reqwest::Client::new()` directly from a page, do not use `document::eval` for fetches.**

The module exposes:

- `ApiError` — typed by status: `NotSignedIn`, `Unauthorized` (401), `BadRequest` (400), `Conflict` (409), `Other(String)`. `Display` produces UI-ready copy.
- `read_token() -> Option<String>` — reads `sharam_id_token` from `localStorage` via `web_sys`.
- `api_url(path) -> Result<String, ApiError>` — prefixes `window.location.origin`. **Required**: WASM `reqwest` parses URLs via `url::Url::parse` and rejects relative paths, so `/api/x` doesn't work without an origin.
- `authed(method, path) -> Result<RequestBuilder, ApiError>` — pre-authed builder with `Authorization: Bearer …` and absolute URL.
- `into_api_error(resp).await` — decodes the gateway's `{ok:false, error:"…"}` envelope into the right `ApiError` variant.

Adding a new endpoint:

```rust
#[derive(Serialize)]
struct CreateThingRequest { /* ... */ }

#[derive(Deserialize)]
struct CreateThingResponse { /* ... */ }

async fn submit_thing(req: CreateThingRequest) -> Result<CreateThingResponse, ApiError> {
    let resp = authed(reqwest::Method::POST, "/api/things")?
        .json(&req).send().await
        .map_err(|e| ApiError::Other(format!("{e:?}")))?;
    if !resp.status().is_success() { return Err(into_api_error(resp).await); }
    resp.json().await.map_err(|e| ApiError::Other(format!("decode: {e}")))
}
```

# Data fetching patterns

**Fetch on mount** — `use_resource`, then match the `Option<Result<T, ApiError>>` it returns. The match must be wrapped in `{…}` inside `rsx!`:

```rust
let ventures = use_resource(|| async move { fetch_ventures().await });
rsx! {
    {match ventures() {
        None => rsx! { /* loading */ },
        Some(Err(ApiError::NotSignedIn | ApiError::Unauthorized)) => rsx! { /* sign-in card */ },
        Some(Err(e)) => { let msg = e.to_string(); rsx! { /* error: {msg} */ } },
        Some(Ok(list)) if list.is_empty() => rsx! { /* empty state */ },
        Some(Ok(list)) => rsx! { /* render */ },
    }}
}
```

**Event-triggered actions** (button clicks etc.) — async closure as the event handler. Dioxus auto-spawns the future. Don't manually `spawn()`; don't reach for `use_resource`.

```rust
let submit = move |_| async move {
    submitting.set(true);
    match submit_thing(req).await {
        Ok(r) => flash.set(Some(format!("Created {}", r.slug))),
        Err(e) => form_error.set(Some(e.to_string())),
    }
    submitting.set(false);
};
```

# `document::eval` — when it's appropriate

Reserved for short, **synchronous** browser-side JS where there's no Rust equivalent: e.g. decoding a JWT payload (`pages/profile.rs`), reading a `<meta>` tag, calling an external script's API (Google Identity Services in `pages/login.rs`).

**Never** route HTTP through `document::eval`. Awaiting `handle.recv()` from inside a hook-spawned future does not reliably wake even when the JS Promise resolves with the correct value — symptom is "Loading…" forever despite a 200 in the network tab. (Background: see `feedback_dioxus_07_fetching.md` in user memory.)

**Never** pass non-string args to `console.log` from JS dispatched via `document::eval` — `dx serve`'s log relay parses them as strings; integers/objects break the runtime message channel and silently kill `recv`. Use `console.log("x=" + JSON.stringify(obj))`.

# Dioxus Dependency

You can add Dioxus to your `Cargo.toml` like this:

```toml
[dependencies]
dioxus = { version = "0.7.1" }

[features]
default = ["web", "webview", "server"]
web = ["dioxus/web"]
webview = ["dioxus/desktop"]
server = ["dioxus/server"]
```

# Launching your application

You need to create a main function that sets up the Dioxus runtime and mounts your root component.

```rust
use dioxus::prelude::*;

fn main() {
	dioxus::launch(App);
}

#[component]
fn App() -> Element {
	rsx! { "Hello, Dioxus!" }
}
```

Then serve with `dx serve`:

```sh
curl -sSL http://dioxus.dev/install.sh | sh
dx serve
```

# UI with RSX

```rust
rsx! {
	div {
		class: "container", // Attribute
		color: "red", // Inline styles
		width: if condition { "100%" }, // Conditional attributes
		"Hello, Dioxus!"
	}
	// Prefer loops over iterators
	for i in 0..5 {
		div { "{i}" } // use elements or components directly in loops
	}
	if condition {
		div { "Condition is true!" } // use elements or components directly in conditionals
	}

	{children} // Expressions are wrapped in brace
	{(0..5).map(|i| rsx! { span { "Item {i}" } })} // Iterators must be wrapped in braces
}
```

# Assets

The asset macro can be used to link to local files to use in your project. All links start with `/` and are relative to the root of your project.

```rust
rsx! {
	img {
		src: asset!("/assets/image.png"),
		alt: "An image",
	}
}
```

## Styles

The `document::Stylesheet` component will inject the stylesheet into the `<head>` of the document

```rust
rsx! {
	document::Stylesheet {
		href: asset!("/assets/styles.css"),
	}
}
```

# Components

Components are the building blocks of apps

* Component are functions annotated with the `#[component]` macro.
* The function name must start with a capital letter or contain an underscore.
* A component re-renders only under two conditions:
	1.  Its props change (as determined by `PartialEq`).
	2.  An internal reactive state it depends on is updated.

```rust
#[component]
fn Input(mut value: Signal<String>) -> Element {
	rsx! {
		input {
            value,
			oninput: move |e| {
				*value.write() = e.value();
			},
			onkeydown: move |e| {
				if e.key() == Key::Enter {
					value.write().clear();
				}
			},
		}
	}
}
```

Each component accepts function arguments (props)

* Props must be owned values, not references. Use `String` and `Vec<T>` instead of `&str` or `&[T]`.
* Props must implement `PartialEq` and `Clone`.
* To make props reactive and copy, you can wrap the type in `ReadOnlySignal`. Any reactive state like memos and resources that read `ReadOnlySignal` props will automatically re-run when the prop changes.

# State

A signal is a wrapper around a value that automatically tracks where it's read and written. Changing a signal's value causes code that relies on the signal to rerun.

## Local State

The `use_signal` hook creates state that is local to a single component. You can call the signal like a function (e.g. `my_signal()`) to clone the value, or use `.read()` to get a reference. `.write()` gets a mutable reference to the value.

Use `use_memo` to create a memoized value that recalculates when its dependencies change. Memos are useful for expensive calculations that you don't want to repeat unnecessarily.

```rust
#[component]
fn Counter() -> Element {
	let mut count = use_signal(|| 0);
	let mut doubled = use_memo(move || count() * 2); // doubled will re-run when count changes because it reads the signal

	rsx! {
		h1 { "Count: {count}" } // Counter will re-render when count changes because it reads the signal
		h2 { "Doubled: {doubled}" }
		button {
			onclick: move |_| *count.write() += 1, // Writing to the signal rerenders Counter
			"Increment"
		}
		button {
			onclick: move |_| count.with_mut(|count| *count += 1), // use with_mut to mutate the signal
			"Increment with with_mut"
		}
	}
}
```

## Context API

The Context API allows you to share state down the component tree. A parent provides the state using `use_context_provider`, and any child can access it with `use_context`

```rust
#[component]
fn App() -> Element {
	let mut theme = use_signal(|| "light".to_string());
	use_context_provider(|| theme); // Provide a type to children
	rsx! { Child {} }
}

#[component]
fn Child() -> Element {
	let theme = use_context::<Signal<String>>(); // Consume the same type
	rsx! {
		div {
			"Current theme: {theme}"
		}
	}
}
```

# Async

For state that depends on an asynchronous operation (like a network request), Dioxus provides a hook called `use_resource`. This hook manages the lifecycle of the async task and provides the result to your component.

* The `use_resource` hook takes an `async` closure. It re-runs this closure whenever any signals it depends on (reads) are updated
* The `Resource` object returned can be in several states when read:
1. `None` if the resource is still loading
2. `Some(value)` if the resource has successfully loaded

```rust
let mut dog = use_resource(move || async move {
	// api request
});

match dog() {
	Some(dog_info) => rsx! { Dog { dog_info } },
	None => rsx! { "Loading..." },
}
```

# Routing

All possible routes are defined in a single Rust `enum` that derives `Routable`. Each variant represents a route and is annotated with `#[route("/path")]`. Dynamic Segments can capture parts of the URL path as parameters by using `:name` in the route string. These become fields in the enum variant.

The `Router<Route> {}` component is the entry point that manages rendering the correct component for the current URL.

You can use the `#[layout(NavBar)]` to create a layout shared between pages and place an `Outlet<Route> {}` inside your layout component. The child routes will be rendered in the outlet.

```rust
#[derive(Routable, Clone, PartialEq)]
enum Route {
	#[layout(NavBar)] // This will use NavBar as the layout for all routes
		#[route("/")]
		Home {},
		#[route("/blog/:id")] // Dynamic segment
		BlogPost { id: i32 },
}

#[component]
fn NavBar() -> Element {
	rsx! {
		a { href: "/", "Home" }
		Outlet<Route> {} // Renders Home or BlogPost
	}
}

#[component]
fn App() -> Element {
	rsx! { Router::<Route> {} }
}
```

```toml
dioxus = { version = "0.7.1", features = ["router"] }
```

# Fullstack

Fullstack enables server rendering and ipc calls. It uses Cargo features (`server` and a client feature like `web`) to split the code into a server and client binaries.

```toml
dioxus = { version = "0.7.1", features = ["fullstack"] }
```

## Server Functions

Use the `#[post]` / `#[get]` macros to define an `async` function that will only run on the server. On the server, this macro generates an API endpoint. On the client, it generates a function that makes an HTTP request to that endpoint.

```rust
#[post("/api/double/:path/&query")]
async fn double_server(number: i32, path: String, query: i32) -> Result<i32, ServerFnError> {
	tokio::time::sleep(std::time::Duration::from_secs(1)).await;
	Ok(number * 2)
}
```

## Hydration

Hydration is the process of making a server-rendered HTML page interactive on the client. The server sends the initial HTML, and then the client-side runs, attaches event listeners, and takes control of future rendering.

### Errors
The initial UI rendered by the component on the client must be identical to the UI rendered on the server.

* Use the `use_server_future` hook instead of `use_resource`. It runs the future on the server, serializes the result, and sends it to the client, ensuring the client has the data immediately for its first render.
* Any code that relies on browser-specific APIs (like accessing `localStorage`) must be run *after* hydration. Place this code inside a `use_effect` hook.
