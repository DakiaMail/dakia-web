import { ApiFailure } from "./api";

// Never echo upstream bodies, private source URLs or arbitrary exception text.
export function imageError(error: unknown): string {
  if (error instanceof ApiFailure) {
    switch (error.code) {
      case "remote_image_rejected": return "Image address blocked by safety checks";
      case "remote_image_unavailable": return "Image host unavailable or returned an error";
      case "remote_image_redirect_blocked": return "Image host returned unsafe, invalid or too many redirects";
      case "remote_image_unsupported_content": return "Response was not a supported PNG, JPEG, GIF or WebP image";
      case "remote_image_too_large": return "Image exceeds the 5 MiB size limit";
      case "image_loading_busy": return "Image loader busy. Try again";
      case "unauthorized": return "Sign in again to load images";
    }
  }
  return "Image request failed. Try again";
}
