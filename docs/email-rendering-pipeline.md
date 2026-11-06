# Email presentation and resource isolation

The Amazon order receipt exposed a shared renderer failure. The old sanitizer removed embedded stylesheets, class/ID selector hooks, responsive rules, most display and dimension properties, individual borders, and `sup`. The reader then changed table display/width and forced cell wrapping. This discarded the information needed to render the receipt correctly. Its product images have inline 64px limits and stylesheet overrides to 122px on wider documents; retaining the width attribute alone was insufficient. Previous button/gutter fixes and containment assertions did not establish overall layout fidelity.

## Rendering boundary

`packages/mail-renderer/src/index.tsx` retains a complete sanitized HTML document, including body/root presentation, safe semantic elements, classes, IDs, data/ARIA attributes, legacy table attributes, and hidden responsive alternatives. Hidden preheaders stay hidden through retained CSS; deleting every hidden subtree would also delete content intended to appear at another breakpoint.

`src/css.ts` parses stylesheets and inline declarations with CSS Tree. Presentation properties are retained rather than reduced to a small layout allowlist. Media/supports/layer/container/scope rules, keyframes, gradients, custom properties, important declarations, dimensions, borders, positioning, flex/grid, and typography can survive inside the isolated document. Custom-property names retain their case. Malformed raw values and executable legacy CSS are discarded. CSS is serialized with HTML style-element termination escaped.

Minimal document defaults precede sender styles. Tables keep their native display and authored geometry; there are no blanket table-width, table-layout or important cell white-space rewrites. When authored content exceeds the reader width, the iframe lays it out at the required width and the parent scales the frame to fit. **Original size** restores unscaled viewing; **Fit to width** returns to the fitted view. Responsive documents that fit already are not scaled. Pane resizing re-evaluates the layout. Image/style updates keep the same sandbox document and do not create browser-history entries.

## Security protections

- DOMPurify still removes scripts, event handlers, forms/inputs, embedded documents, SVG/MathML and active document metadata. Selector support does not place sender content into the app DOM.
- The iframe remains an opaque origin, without `allow-same-origin`. Sender positioning cannot cover the parent app. Only the app's nonce-authorized size reporter runs.
- CSP denies network by default, denies fonts/base URLs/forms, and permits only mediated raster data images. Imports, remote font definitions, resource-bearing image-set/image/paint/element/attr functions, unparsed CSS values and executable legacy properties do not pass through.
- Image `src`, supported CSS image URLs and legacy `background` attributes share URL normalization and the existing authenticated image service. Sender data URLs are not accepted as a shortcut. Unrequested or failed image URLs never remain as browser-fetchable fallbacks. Existing image permission and server address/TLS/redirect/type/size checks still apply.
- Hidden/pixel preferences continue to filter explicitly hidden or tiny images; hidden ancestors and inline/legacy backgrounds are also checked. This is not universal tracking detection. Stylesheet-only or conditional hiding cannot always be determined by synchronous image discovery, and image permission can allow tracking images just as it could before.
- External links retain protocol filtering, `_blank`, `noopener noreferrer` and no-referrer behavior. Local fragment links remain inside the email. Resize messages require the correct source, opaque origin, channel and current viewport, with finite bounded dimensions.

## Verification

Build first, then run the production Chrome harness:

```sh
npm test
npm run build
CHROME_PATH=/path/to/google-chrome npm run test:browser --workspace @dakia/web -- \
  --config playwright.renderer.config.ts \
  email-layouts.spec.ts email-buttons.spec.ts email-gutters.spec.ts \
  html-contrast.spec.ts newsletter-fidelity.spec.ts mobile-reader.spec.ts \
  images.spec.ts image-errors.spec.ts mail.spec.ts mobile-back.spec.ts reader-return.spec.ts
cargo test --locked -p dakia-server images::
```

Use a supported Node version from the repository requirements. The new browser cases cover responsive receipts, fixed-width fit/original viewing and resizing, public MailChimp/colspan layouts, complete scrolling, CSS image consent, active and malformed markup/CSS, opaque-origin isolation and forged size reports. Existing cases cover real mouse/keyboard popup navigation, image failures/retries, hidden images, light/dark panels, quote/reply behavior and browser history.

The optional `DAKIA_PRIVATE_AMAZON` directory contains owner-approved `amazon.json` (`html` and `text`) and `amazon-images.json` (URL-to-raster-data mapping). These files and the resulting screenshots remain private and uncommitted. All account/header metadata in the harness is fictional. Browser requests are intercepted; the test never contacts Amazon links, tracking pixels or provider APIs. Static original artwork is supplied from the private mapping.

At app viewports 320, 390, 600, 900, 1480 and 1920, the private cases check decoded product dimensions against the original HTML rendered at the same document width with networking denied, preserve price superscripts and tracker geometry, and capture the complete message plus app views. These are Chrome checks, not physical-device Safari/Android acceptance or a universal pixel-fidelity claim. CID attachments, remote fonts, SVG and externally imported stylesheets are not enabled by this change.

### Verified release checks

On 6 November 2026, the isolated source passed TypeScript/Vite build, all 179 web unit tests, all 164 selected production Chrome scenarios, and 10 server image-security tests. Four explicitly opt-in image-network/diagnostic tests remained skipped. The focused fit/navigation and viewport-height cases also passed three repeated runs. No real mailbox mutations were used for acceptance.
