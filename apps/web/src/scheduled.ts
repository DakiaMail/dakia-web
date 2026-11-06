import type { Submission } from "@dakia/api-types";

export function submissionLabel(submission: Submission) {
  switch (submission.state) {
    case "scheduled": return "Scheduled";
    case "claimed": case "submitting": return "Sending…";
    case "accepted": return submission.filing_status === "failed" ? "Sent, but the copy could not be saved in Sent." : "Sent";
    case "cancelled": return "Send cancelled";
    case "failed": return "Not sent. Check the account connection and recipients before trying again.";
    case "delivery_unknown": return "Delivery unconfirmed. Check Sent before sending again.";
  }
}
export function scheduledTime(submission: Submission) {
  const options: Intl.DateTimeFormatOptions = { month: "short", day: "numeric", year: "numeric", hour: "numeric", minute: "2-digit" };
  try { return new Date(submission.due_at).toLocaleString(undefined, { ...options, timeZone: submission.timezone }); }
  catch { return new Date(submission.due_at).toLocaleString(undefined, options); }
}
