import { test, expect } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { accounts, content, page as mail } from "./fixtures";

// Synthetic sender HTML only. No private email or live mailbox APIs.
const html = `<div style="background-color:#ffffff;padding:20px">
  <h1 style="color:#000000">Example Studio</h1>
  <p>Thanks for your submission.</p>
  <p><strong>Your project is ready for review.</strong></p>
  <p><a href="https://example.org/review">Review the submission</a></p>
  <table style="background-color:#f5f5f5"><tr><td style="padding:12px">Table with inherited text</td></tr></table>
</div>
<div style="color:#f5f5f5;background-color:#202126;padding:16px">
  <p>Sender dark panel <span>with nested inheritance</span></p>
  <p><a style="color:#a3dfff" href="https://example.org/help">Sender styled link</a></p>
  <p><a href="https://example.org/nested">Inherited dark-panel link</a></p>
  <div style="color:#222222;background-color:#ffffff;padding:8px">Nested light panel</div>
</div>
<p style="color:#444444">Sender foreground only</p>
<p style="background-color:#eeeeee">Sender background only</p>
<p>Unstyled footer <a href="mailto:help@example.org">Contact support</a></p>`;

for (const width of [390, 1480]) {
  for (const theme of ["light", "dark"]) {
    for (const plain of [false, true]) {
      test(`${width} ${theme} ${plain ? "plain text" : "sender HTML"} readable isolated body`, async ({ page }) => {
        await page.setViewportSize({ width, height: 980 });
        const external: string[] = [];
        await page.route("**/*", async (route) => {
          const url = new URL(route.request().url());
          if (url.origin !== "http://127.0.0.1:4173") {
            external.push(url.origin);
            return route.abort();
          }
          if (!url.pathname.startsWith("/api/")) return route.continue();
          const path = url.pathname;
          const respond = (json: unknown) => route.fulfill({ json });
          if (path === "/api/session") return respond({ user_id: "fixture", csrf_token: "fixture" });
          if (path === "/api/accounts") return respond(accounts);
          if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": connected\n\n" });
          if (path === "/api/search" || path.endsWith("/messages")) return respond(mail);
          if (path.endsWith("/conversation")) return respond({ rows: [content.message], continuation: null, complete: true });
          if (path.endsWith("/messages/m0")) return respond({ ...content, detected_language: "en", html: plain ? "" : html, text: "Plain text message\nSecond line <safe> & readable." });
          if (path.endsWith("/operations")) {
            const op = route.request().postDataJSON();
            return respond({ ...op, status: "applied", revision: op.expected_revision + 1 });
          }
          if (path.endsWith("/settings")) return respond({});
          if (path.endsWith("/folders")) return respond(["INBOX"]);
          if (path.endsWith("/unsubscribe")) return respond(null);
          if (path.endsWith("/scheduled") || path.endsWith("/submissions")) return respond([]);
          return respond(null);
        });
        await page.addInitScript((theme) => localStorage.setItem("dakia-theme", theme), theme);
        await page.goto("/");
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        await page.getByRole("button", { name: /Maya Chen.*A few thoughts/ }).click();
        const frame = page.frameLocator('iframe[title="Message body"]');
        const body = frame.locator("body");
        await expect(body).toContainText(plain ? "Plain text message" : "Thanks for your submission.");
        await expect(body).toHaveCSS("color", plain && theme === "dark" ? "rgb(241, 242, 244)" : "rgb(32, 35, 43)");
        await expect(frame.locator("html")).toHaveCSS("background-color", plain && theme === "dark" ? "rgb(32, 33, 38)" : "rgb(255, 255, 255)");
        await expect(frame.locator("html")).toHaveCSS("color-scheme", plain && theme === "dark" ? "dark" : "light");
        if (plain) {
          await expect(body).toContainText("<safe> & readable.");
          await expect(body.locator("safe")).toHaveCount(0);
        } else {
          await expect(body.locator("div").first()).toHaveCSS("padding", "20px");
          await expect(body.locator("div").first()).toHaveCSS("background-color", "rgb(255, 255, 255)");
          await expect(frame.getByText("with nested inheritance").locator("../..")).toHaveCSS("background-color", "rgb(32, 33, 38)");
          await expect(frame.getByText("Thanks for your submission.")).toHaveCSS("color", "rgb(32, 35, 43)");
          await expect(frame.getByText("Table with inherited text")).toHaveCSS("color", "rgb(32, 35, 43)");
          await expect(frame.getByText("with nested inheritance")).toHaveCSS("color", "rgb(245, 245, 245)");
          await expect(frame.getByText("Nested light panel")).toHaveCSS("color", "rgb(34, 34, 34)");
          await expect(frame.getByText("Sender foreground only")).toHaveCSS("color", "rgb(68, 68, 68)");
          await expect(frame.getByText("Sender background only")).toHaveCSS("color", "rgb(32, 35, 43)");
          await expect(frame.getByRole("link", { name: "Review the submission" })).toHaveCSS("color", "rgb(32, 35, 43)");
          await expect(frame.getByRole("link", { name: "Review the submission" })).toHaveCSS("text-decoration-line", "underline");
          await expect(frame.getByRole("link", { name: "Inherited dark-panel link" })).toHaveCSS("color", "rgb(245, 245, 245)");
          await expect(frame.getByRole("link", { name: "Sender styled link" })).toHaveCSS("color", "rgb(163, 223, 255)");
          await expect(frame.getByRole("link", { name: "Contact support" })).toHaveAttribute("rel", "noopener noreferrer");
        }
        await expect(page.locator('iframe[title="Message body"]')).toHaveAttribute("sandbox", "allow-scripts allow-popups allow-popups-to-escape-sandbox");
        const dimensions = await body.evaluate((node) => ({ scroll: node.scrollWidth, width: node.clientWidth }));
        expect(dimensions.scroll).toBeLessThanOrEqual(dimensions.width + 1);
        expect(external).toEqual([]);
        mkdirSync(resolve("../../docs/screenshots/html-contrast"), { recursive: true });
        await page.screenshot({ path: resolve(`../../docs/screenshots/html-contrast/${width}-${theme}-${plain ? "plain" : "html"}.png`), fullPage: true });
      });
    }
  }
}
