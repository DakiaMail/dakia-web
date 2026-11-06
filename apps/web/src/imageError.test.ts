import { describe, expect, it } from "vitest";
import { ApiFailure } from "./api";
import { imageError } from "./imageError";

describe("image failure explanations", () => {
  it("distinguishes upstream failures from content and security decisions", () => {
    const codes = ["remote_image_rejected", "remote_image_unavailable", "remote_image_redirect_blocked", "remote_image_unsupported_content", "remote_image_too_large", "image_loading_busy"];
    const messages = codes.map(code => imageError(new ApiFailure(400, code, "private upstream body https://private.invalid/token")));
    expect(new Set(messages).size).toBe(codes.length);
    expect(messages.join(" ")).not.toMatch(/private|token|https:/);
    expect(messages[1]).toContain("host unavailable");
    expect(messages[2]).toContain("unsafe, invalid or too many redirects");
  });
  it("does not present unexpected failures as safety rejections or expose their text", () => {
    expect(imageError(new Error("private URL"))).toBe("Image request failed. Try again");
    expect(imageError(new ApiFailure(500, "unknown", "private URL"))).toBe("Image request failed. Try again");
  });
});
