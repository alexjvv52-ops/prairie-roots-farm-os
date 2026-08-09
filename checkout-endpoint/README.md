# Checkout endpoint

Optional. You do not need this folder unless you sell online through the shop page.

This is a Cloudflare Worker that creates Stripe Checkout sessions. The desktop app is complete without it. It holds no farm data and stores nothing.

## Secrets

Set these with Wrangler. Never commit them.

- `STRIPE_RESTRICTED_KEY`
- `ALLOWED_ORIGIN`
- `SUCCESS_URL`
- `CANCEL_URL`

## Test

```bash
cd checkout-endpoint
npm ci
node --test
```

## Deploy

Install Wrangler and authenticate with Cloudflare, then:

```bash
npx wrangler secret put STRIPE_RESTRICTED_KEY
npx wrangler secret put ALLOWED_ORIGIN
npx wrangler secret put SUCCESS_URL
npx wrangler secret put CANCEL_URL
npx wrangler deploy
```
