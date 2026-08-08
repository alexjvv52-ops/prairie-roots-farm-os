# Prairie Roots — checkout endpoint

A **stateless** Cloudflare Worker that turns a cart of Stripe Price IDs into a Checkout Session URL.

It stores nothing, reads no farm data, and is not the system of record. If it is down, Farm OS keeps working — the desktop app never calls it.

## Rule

The endpoint **never trusts a client-supplied amount**. Clients send Price IDs + quantities; Stripe prices the cart. The posted `total` is only used to detect a stale page (`409` if it disagrees with live Price objects).

## Deploy (Cloudflare Workers)

1. Install Wrangler once: `npm i -g wrangler` (or use `npx wrangler`).
2. From this folder, set secrets / vars (never commit them):

```bash
npx wrangler secret put STRIPE_RESTRICTED_KEY
# paste a test-mode restricted key: rk_test_…
```

```bash
npx wrangler secret put ALLOWED_ORIGIN
# Exact Origin(s), comma-separated if more than one.
# e.g. https://your-shop-host.example
# or   http://localhost:5500,http://127.0.0.1:5500,https://your-shop-host.example

npx wrangler secret put SUCCESS_URL
# e.g. https://your-shop-host.example/?paid=1

npx wrangler secret put CANCEL_URL
# e.g. https://your-shop-host.example/?cancelled=1
```

Or for local dev, create `.dev.vars` (gitignored):

```
STRIPE_RESTRICTED_KEY=rk_test_…
ALLOWED_ORIGIN=http://localhost:5500,http://127.0.0.1:5500
SUCCESS_URL=http://127.0.0.1:5500/?paid=1
CANCEL_URL=http://127.0.0.1:5500/?cancelled=1
```

3. Deploy:

```bash
npx wrangler deploy
```

Dry-run (no publish):

```bash
npx wrangler deploy --dry-run
```

### Key requirements

- Must be a **restricted** key (`rk_…`). Secret keys (`sk_…`) are refused.
- Must be **test mode** (`rk_test_…`) while `ALLOW_LIVE_KEYS` is `false` in `src/handler.js`. Flipping that flag is a deliberate code change, reviewed and redeployed — never an env var or request parameter.

## Environment variables

| Name | Purpose |
|------|---------|
| `STRIPE_RESTRICTED_KEY` | Test-mode restricted key (`rk_test_…`) |
| `ALLOWED_ORIGIN` | Exact `Origin`(s) allowed to call the Worker — comma-separated list, exact equality only; matched Origin is echoed back. Never `*` |
| `SUCCESS_URL` | Stripe Checkout `success_url` |
| `CANCEL_URL` | Stripe Checkout `cancel_url` |

## Request

`POST /` with JSON:

```json
{
  "reference": "9f1c-example-uuid",
  "harvestDate": "2026-08-14",
  "currency": "cad",
  "total": 4500,
  "lines": [
    { "priceId": "price_…", "quantity": 3 },
    { "priceId": "price_…", "quantity": 2 }
  ]
}
```

Success: `200 {"url":"https://checkout.stripe.com/…"}` — URL only.

## Tests

No runtime dependencies. From this folder:

```bash
node --test
```

## Netlify Functions (drop-in alternative)

`src/handler.js` exports `handleCheckout(request, env, fetchImpl)` as a pure function. A Netlify Function can wrap the same handler with the platform’s `Request`/`Response` and `fetch` — no Cloudflare-specific APIs are used inside the handler.
