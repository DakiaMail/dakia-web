// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Translation } from "./Translation";
import { messages, content } from "../browser/fixtures";
import { extraApi } from "./extra-api";
import type { ModelJob } from "@dakia/api-types";
import registry from "../../../workers/translation/models.json";

// jsdom does not implement the reader's standard browser resize API.
vi.stubGlobal("ResizeObserver", class { observe() {} disconnect() {} });
afterEach(() => { cleanup(); vi.restoreAllMocks(); });
it("does not offer Estonian translation for unknown or English regional content", () => {
  const request = vi.spyOn(extraApi, "translate");
  const client = new QueryClient();
  const ui = (detectedLanguage: string | null) => <QueryClientProvider client={client}><Translation message={messages[0]} html={content.html} text={content.text} dark={false} detectedLanguage={detectedLanguage} /></QueryClientProvider>;
  const view = render(ui(null));
  expect(screen.queryByRole("button", { name: "Translate to English" })).toBeNull();
  view.rerender(ui("en-US"));
  expect(screen.queryByRole("button", { name: "Translate to English" })).toBeNull();
  expect(request).not.toHaveBeenCalled();
  client.clear();
});

it("offers only registered sources, normalized without an Estonian fallback", () => {
  const request = vi.spyOn(extraApi, "translate");
  const client = new QueryClient();
  const ui = (detectedLanguage: string | null) => <QueryClientProvider client={client}><Translation message={messages[0]} html={content.html} text={content.text} dark={false} detectedLanguage={detectedLanguage} /></QueryClientProvider>;
  const view = render(ui(null));
  for (const value of [null, "en", "en-US", "en_GB", "xx", "rus", ""]) {
    view.rerender(ui(value));
    expect(screen.queryByLabelText("Translate from")).toBeNull();
  }
  for (const model of registry.models) {
    view.rerender(ui(`${model.source}-XX`));
    expect((screen.getByLabelText("Translate from") as HTMLSelectElement).value).toBe(model.source);
  }
  expect(request).not.toHaveBeenCalled();
  client.clear();
});

const job: ModelJob = { id: "j1", account_id: "work", message_id: "m0", content_revision: 1, model_revision: "fixture", kind: "translate", status: "succeeded", deadline: "2026-11-03T00:00:00Z" };
it("translates only on click, shows original, resets source selection and ignores discarded completion", async () => {
  let finish!: (job: ModelJob) => void;
  const translate = vi.spyOn(extraApi, "translate").mockImplementationOnce(() => new Promise(resolve => { finish = resolve; })).mockResolvedValue(job);
  vi.spyOn(extraApi, "job").mockResolvedValue(job);
  vi.spyOn(extraApi, "translation").mockResolvedValue({ ...content, text: "Translated synthetic message", html: "<p>Translated synthetic message</p>" });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const ui = (language: string | null, account = "work", html = content.html) => <QueryClientProvider client={client}><Translation message={{ ...messages[0], account_id: account }} html={html} text={content.text} dark={false} detectedLanguage={language} /></QueryClientProvider>;
  const view = render(ui("et"));
  expect(translate).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Translate to English" }));
  expect(translate).toHaveBeenCalledWith(messages[0], "et");
  view.rerender(ui("ar", "personal"));
  expect((screen.getByLabelText("Translate from") as HTMLSelectElement).value).toBe("ar");
  await act(async () => { finish(job); });
  expect(screen.queryByRole("button", { name: "Show original" })).toBeNull();
  fireEvent.change(screen.getByLabelText("Translate from"), { target: { value: "ru" } });
  fireEvent.click(screen.getByRole("button", { name: "Translate to English" }));
  await screen.findByRole("button", { name: "Show original" });
  expect(translate).toHaveBeenLastCalledWith(expect.objectContaining({ account_id: "personal" }), "ru");
  expect(screen.getByTitle("Translated message")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Show original" }));
  expect(screen.getByTitle("Message body")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Show translation" }));
  expect(translate).toHaveBeenCalledTimes(2);
  view.rerender(ui("ja", "personal", "<p>Revised body</p>"));
  expect((screen.getByLabelText("Translate from") as HTMLSelectElement).value).toBe("ja");
  expect(screen.queryByTitle("Translated message")).toBeNull();
  view.rerender(ui(null));
  expect(screen.queryByLabelText("Translate from")).toBeNull();
  await waitFor(() => expect(screen.queryByRole("status")).toBeNull());
  client.clear();
});
