# Dakia demo

Try [demo.dakiamail.com](https://demo.dakiamail.com) with fictional Work and Personal mailboxes. The demo uses the real frontend with a mailbox stored in your browser. It doesn't connect to an email provider or send real mail.

Explore Personal and Automated, type filters, Needs attention, search, conversations, drafts, snooze, cleanup, and scheduling. Actions change only this browser's demo mailbox. Translation uses a prepared example; it doesn't run the translation engine. Account connection and other actions that need a real server are unavailable.

Open the demo with `?reset=1` to restore its starting mailbox. Demo data can survive reloads in IndexedDB; clearing site data also resets it.

## Run locally

From the repository root, with Node.js 24 and npm:

```sh
npm ci --ignore-scripts
npm run dev --workspace @dakia/web -- --mode demo
```

Open the loopback address printed by Vite. To build the standalone demo:

```sh
npm run build:demo
npm exec --workspace @dakia/web -- vite preview --outDir dist-demo --host 127.0.0.1
```

The output is `apps/web/dist-demo`. Keep this build separate from the normal server frontend. It needs no mail credentials, server database, or backend API.

## Checks and publishing

`npm run test:demo` checks the demo journeys; `npm run test:parity` compares supported demo and API-fixture scenes. The Demo GitHub Actions workflow builds and checks the static app before publishing main. See [the workflow](../.github/workflows/demo.yml) for hosting configuration.

Fictional content lives in `apps/web/src/demo/seed.ts`; the local mailbox transport is in `apps/web/src/demo/mailbox.ts`. These scenes demonstrate frontend behavior, not provider connectivity or delivery.
