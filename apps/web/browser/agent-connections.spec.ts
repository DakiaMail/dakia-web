import { test, expect } from "@playwright/test";
import { accounts, page as mail } from "./fixtures";

for (const { theme, width } of [{ theme: "light", width: 1440 }, { theme: "dark", width: 390 }]) {
  test(`inline agent setup in ${theme} Workspace at ${width}px`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 850 });
    await page.addInitScript(value => localStorage.setItem("dakia-theme", value), theme);
    let created = 0;
    let rows = [{ id: "existing", name: "Existing agent", allow_send: false, allow_delete: false, revoked: false, created_at: 1 }];
    await page.route("**/api/**", route => {
      const path = new URL(route.request().url()).pathname;
      const method = route.request().method();
      if (path === "/api/session") return route.fulfill({ json: { user_id: "agent-fixture", csrf_token: "synthetic" } });
      if (path === "/api/accounts") return route.fulfill({ json: accounts });
      if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
      if (path.endsWith("/sync-active")) return route.fulfill({ json: false });
      if (path === "/api/search" || path.endsWith("/messages")) return route.fulfill({ json: mail });
      if (path === "/api/agent-connections" && method === "POST") {
        created += 1;
        const connection = { id: "manual", name: route.request().postDataJSON().name, allow_send: false, allow_delete: false, revoked: false, created_at: 2 };
        rows = [...rows, connection];
        return route.fulfill({ json: { connection, token: "fixture-only-token" } });
      }
      if (path === "/api/agent-connections/existing" && method === "PUT") {
        const update = route.request().postDataJSON();
        expect(Object.keys(update).sort()).toEqual(["allow_delete", "allow_send", "name"]);
        rows[0] = { ...rows[0], ...update };
        return route.fulfill({ json: rows[0] });
      }
      if (path === "/api/agent-connections/manual" && method === "DELETE") {
        rows = rows.filter(row => row.id !== "manual");
        return route.fulfill({ status: 204 });
      }
      return route.fulfill({ json: path === "/api/agent-connections" ? rows : [] });
    });

    await page.goto("/");
    if (width < 600) await page.getByRole("button", { name: "Open navigation" }).click();
    await page.getByRole("button", { name: "Manage workspace" }).click();
    const agents = page.getByRole("region", { name: "Agent connections" });
    const chatgpt = agents.getByRole("radio", { name: "ChatGPT" });
    await expect(chatgpt).toBeChecked();
    await expect(agents.getByText("Add custom MCP server")).toBeVisible();
    await expect(agents.getByRole("button", { name: "Create token" })).toHaveCount(0);
    await expect(agents.getByRole("button", { name: "Copy URL" })).toBeVisible();
    await agents.screenshot({ animations: "disabled", path: testInfo.outputPath("agent-settings.png") });
    await chatgpt.focus();
    await page.keyboard.press("ArrowRight");
    await expect(agents.getByRole("radio", { name: "Claude" })).toBeChecked();
    await expect(agents.getByText("Add custom connector")).toBeVisible();
    await page.keyboard.press("ArrowRight");
    await expect(agents.getByRole("radio", { name: "Hermes Agent" })).toBeChecked();
    await expect(agents.getByText(/hermes mcp add dakia --url .* --auth oauth/)).toBeVisible();
    await page.keyboard.press("ArrowRight");
    await expect(agents.getByRole("radio", { name: "Other AI agent" })).toBeChecked();
    expect(created).toBe(0);

    await agents.getByRole("button", { name: "Check server URL" }).click();
    await expect(agents.getByRole("status").filter({ hasText: "Use a public HTTPS address" })).toBeVisible();
    await agents.getByRole("button", { name: "Edit Existing agent" }).click();
    await expect(agents.getByRole("textbox", { name: "Connection name" })).toBeFocused();
    await agents.getByRole("checkbox", { name: "Allow sending" }).check();
    await agents.getByRole("button", { name: "Save changes" }).click();
    await expect(agents.getByRole("button", { name: "Edit Existing agent" })).toBeFocused();
    await expect(agents.getByText("Send mail", { exact: true })).toBeVisible();

    await agents.getByRole("button", { name: "Use a token instead" }).click();
    await agents.getByRole("textbox", { name: "Connection name" }).fill("Manual agent");
    await agents.getByRole("button", { name: "Create token" }).click();
    await expect(agents.getByText("Authorization: Bearer fixture-only-token")).toBeVisible();
    await agents.getByRole("button", { name: "Close manual setup" }).click();
    await expect(agents.getByText("Authorization: Bearer fixture-only-token")).toHaveCount(0);
    await agents.getByRole("button", { name: "Revoke Manual agent" }).click();
    await page.getByRole("dialog").getByRole("button", { name: "Revoke connection" }).click();
    await expect(agents.getByRole("heading", { name: "Agent connections" })).toBeFocused();
    await expect(agents.getByRole("article", { name: "Manual agent" })).toHaveCount(0);
    expect(await agents.evaluate(element => element.scrollWidth <= element.clientWidth + 1)).toBe(true);
  });
}
