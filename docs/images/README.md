# README inbox tour

Captured on 2026-11-07 from dakia-web revision `fcfed065847dcb360a7e4626dc7f8f5faf2dd6cf`, using the unmodified Vite frontend and the fictional accounts, messages, and message content in `apps/web/browser/fixtures.ts`.

An isolated Playwright browser intercepted `/api` requests with those fixtures. No real mailbox, personal browser profile, email delivery, or provider connection was used. Other external network requests were blocked. The screenshots demonstrate the real frontend, not backend behavior.

- Browser: Chromium 156.0.8078.79, English, Europe/Tallinn, scale 1.
- Clock fixed to 2026-10-29T08:15:00Z.
- Desktop browser viewport: 1440 × 1000. Mobile browser viewport: 390 × 844, not a native phone capture.
- Scene: Smart inbox across fictional Work and Personal accounts, then Maya Chen's “A few thoughts on the launch” opened through the real message control. The reader greeting and closing were visible before capture.
- Static exports: `inbox-desktop.webp` (1440 × 1000, 101638 bytes) and `inbox-mobile.webp` (390 × 844, 28546 bytes), Pillow 12.3.0, WebP quality 88, method 6, no source metadata.
- `inbox-tour.gif`: four captured UI states, in order: desktop inbox, desktop reader, mobile inbox, mobile reader. Frames are proportionally reduced and centered on a neutral 960 × 670 canvas, never stretched or repainted. Holds: 1.8, 2.6, 1.8, 2.6 seconds, 8.8 seconds total, looping. 192-color GIF, 254032 bytes.
- All four original scenes were visually reviewed. Normal scroll boundaries and the mobile floating compose button are retained as the app renders them. Names, example-domain addresses, text, and attachment metadata are fictional fixtures.

Static alternatives are linked next to the animation in the README.
