import { useEffect, useId, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Languages } from "lucide-react";
import type { Message, ModelJob } from "@dakia/api-types";
import { MailReader, remoteImageSources, sanitizeMail } from "@dakia/mail-renderer";
import { api } from "./api";
import { imageError } from "./imageError";
import { extraApi } from "./extra-api";
import { ErrorNotice } from "./ui";
import { deviceTranslator, DeviceTranslationError, formatBytes, type DownloadProgress } from "./deviceTranslation";
import { useTranslationPolicy } from "./translationPolicy";
import "./translation.css";
import registry from "../../../workers/translation/models.json";
type ImageResult = { loaded: Record<string, string>; failed: Record<string, string> };
type Props = {
  message: Message;
  html: string;
  text: string;
  dark: boolean;
  detectedLanguage?: string | null;
  alwaysLoadImages?: boolean;
  blockHiddenImages?: boolean;
};
const languages = registry.models.filter(model => model.target === "en");
function sourceLanguage(value?: string | null) {
  const base = value?.trim().toLowerCase().split(/[-_]/)[0];
  return languages.some(model => model.source === base) ? base! : null;
}
export function Translation(props: Props) {
  // Reset before paint, including same-ID body revisions and account switches.
  // A discarded child's async completion cannot update the new reader.
  const source = sourceLanguage(props.detectedLanguage);
  const [scope, setScope] = useState({ account: props.message.account_id, id: props.message.id, html: props.html, text: props.text, source, revision: 0 });
  if (scope.account !== props.message.account_id || scope.id !== props.message.id || scope.html !== props.html || scope.text !== props.text || scope.source !== source) {
    setScope({ account: props.message.account_id, id: props.message.id, html: props.html, text: props.text, source, revision: scope.revision + 1 });
  }
  return <TranslationBody key={scope.revision} {...props} source={source} />;
}
function TranslationBody({
  message,
  html,
  text,
  dark,
  source,
  alwaysLoadImages = true,
  blockHiddenImages = false,
}: Props & { source: string | null }) {
  const policy = useTranslationPolicy();
  const owner = useId();
  const controller = useRef<AbortController | null>(null);
  const translateButton = useRef<HTMLButtonElement>(null);
  const [deviceResult, setDeviceResult] = useState<{ html: string; text: string } | null>(null);
  const [download, setDownload] = useState<DownloadProgress | null>(null);
  const [hostFallback, setHostFallback] = useState(false);
  useEffect(() => () => { controller.current?.abort(); controller.current = null; deviceTranslator.releaseOwner(owner); }, [owner]);
  useEffect(() => { controller.current?.abort(); controller.current = null; deviceTranslator.releaseOwner(owner); setDeviceResult(null); setJob(null); setShow(false); setBusy(false); setDownload(null); }, [policy.data?.mode, owner]);
  const [language, setLanguage] = useState(source || "");
  const [job, setJob] = useState<ModelJob | null>(null);
  const [show, setShow] = useState(false);
  const [error, setError] = useState<unknown>();
  const [busy, setBusy] = useState(false);
  const [manuallyLoadImages, setManuallyLoadImages] = useState(false);
  const client = useQueryClient();
  const progress = useQuery({
    queryKey: ["translation-job", message.account_id, job?.id],
    queryFn: () => extraApi.job(message.account_id, job!.id),
    enabled: !!job,
    refetchInterval: (q) =>
      ["pending", "running"].includes(q.state.data?.status || job?.status || "")
        ? 1500
        : false,
  });
  const current = progress.data || job;
  const translated = useQuery({
    queryKey: ["translated", message.account_id, message.id, language, job?.id],
    queryFn: () => extraApi.translation(message, language),
    enabled: current?.status === "succeeded",
  });
  const result = deviceResult || translated.data;
  const imageHtml = show && result ? result.html : html;
  const imageSources = useMemo(() => remoteImageSources(imageHtml, blockHiddenImages), [imageHtml, blockHiddenImages]);
  const allowImages = alwaysLoadImages || manuallyLoadImages;
  const imageKey = ["remote-images", message.account_id, message.id, imageSources];
  const imageQuery = useQuery({
    queryKey: imageKey,
    enabled: allowImages && imageSources.length > 0,
    staleTime: Infinity,
    retry: false,
    queryFn: async ({ signal }) => {
      // Let an immediately discarded mount/navigation abort before a tracking
      // image is requested, including React's development StrictMode probe.
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
      signal.throwIfAborted();
      // One bounded server response at a time, with incremental display. There
      // is no message-wide count cap; retry only sources that did not succeed.
      const loaded = { ...client.getQueryData<ImageResult>(imageKey)?.loaded };
      const failed: Record<string, string> = {};
      for (const url of imageSources) {
        signal.throwIfAborted();
        if (loaded[url]) continue;
        try {
          loaded[url] = await api.loadImage(message, { url }, signal);
        } catch (error) {
          signal.throwIfAborted();
          failed[url] = imageError(error);
        }
        client.setQueryData<ImageResult>(imageKey, { loaded: { ...loaded }, failed: { ...failed } });
      }
      return { loaded, failed };
    },
  });
  useEffect(() => {
    if (!allowImages) void client.cancelQueries({ queryKey: ["remote-images", message.account_id, message.id] });
  }, [allowImages, client, message.account_id, message.id]);
  const images = allowImages ? imageQuery.data?.loaded : undefined;
  const failures = allowImages ? Object.entries(imageQuery.data?.failed || {}) : [];
  const failedImages = failures.length;
  function cancel() {
    controller.current?.abort(); controller.current = null;
    setBusy(false); setDownload(null); setError(undefined);
    requestAnimationFrame(() => translateButton.current?.focus());
  }
  async function translate() {
    if (!source || !language || busy || !policy.data || current?.status === "pending" || current?.status === "running") return;
    if (result) { setShow(!show); return; }
    const abort = new AbortController(); controller.current = abort;
    setBusy(true); setError(undefined); setHostFallback(false);
    try {
      if (policy.data.mode !== "host") {
        try {
          const output = await deviceTranslator.translate(owner, language, html ? sanitizeMail(html) : text, Boolean(html), abort.signal, value => {
            if (controller.current === abort) setDownload(value);
          });
          if (controller.current !== abort) return;
          setDeviceResult(html ? { html: sanitizeMail(output), text: "" } : { html: "", text: output });
          setShow(true); return;
        } catch (error) {
          abort.signal.throwIfAborted();
          if (error instanceof DOMException && error.name === "AbortError") throw error;
          if (policy.data.mode !== "automatic" || error instanceof DeviceTranslationError && !error.fallback) throw error;
          setDownload(null); setHostFallback(true);
        }
      }
      abort.signal.throwIfAborted();
      const job = await extraApi.translate(message, language);
      if (controller.current === abort) { setJob(job); setShow(true); }
    } catch (error) {
      if (controller.current === abort && !abort.signal.aborted && !(error instanceof DOMException && error.name === "AbortError")) setError(error);
    } finally {
      if (controller.current === abort) { setBusy(false); setDownload(null); }
    }
  }
  return (
    <div className="translation">
      <div className="translation-tools">
        {imageSources.length > 0 && (!images || imageQuery.error || (failedImages > 0 && Object.keys(images).length === 0)) && <button title={imageQuery.error || failedImages > 0 ? "Retry loading images" : "Load images"} disabled={imageQuery.isFetching} onClick={() => {
          setManuallyLoadImages(true);
          if (allowImages) void imageQuery.refetch();
        }}>{imageQuery.isFetching ? "Loading images…" : imageQuery.error || failedImages > 0 ? "Retry loading images" : "Load images"}</button>}
        {source && <><select title="Translate from"
          aria-label="Translate from"
          value={language}
          disabled={
            busy ||
            current?.status === "running" ||
            current?.status === "pending"
          }
          onChange={(e) => {
            setLanguage(e.target.value);
            setJob(null);
            setDeviceResult(null);
            setShow(false);
          }}
        >
          {languages.map(({ source: value, sourceName: label }) => (
            <option key={value} value={value}>
              {label}
            </option>
          ))}
        </select>
        <button ref={translateButton} aria-disabled={busy || !policy.data || current?.status === "pending" || current?.status === "running"} title={result ? show ? "Show original" : "Show translation" : "Translate to English"}
          onClick={() => void translate()}
        >
          <Languages size={15} />
          {result
            ? show
              ? "Show original"
              : "Show translation"
            : "Translate to English"}
        </button></>}
      </div>
      {busy && download && <div className="translation-download">
        <div className={`translation-progress-ring ${download.stage !== "downloading" ? "is-indeterminate" : ""}`} role="progressbar" aria-label={download.stage === "downloading" ? `Downloading ${languages.find(m => m.source === language)?.sourceName}` : "Preparing translation"} aria-valuemin={0} aria-valuemax={100} aria-valuenow={download.stage === "downloading" && download.total > 0 ? Math.min(100, Math.round(download.downloaded / download.total * 100)) : undefined}>
          <svg viewBox="0 0 40 40" aria-hidden="true"><circle cx="20" cy="20" r="16" pathLength="100" /><circle className="translation-progress-value" cx="20" cy="20" r="16" pathLength="100" strokeDasharray="100" strokeDashoffset={download.stage === "downloading" && download.total ? 100 - download.downloaded / download.total * 100 : 70} /></svg>
        </div>
        <div className="translation-download-copy"><span role="status">{download.stage === "downloading" ? `Downloading ${languages.find(m => m.source === language)?.sourceName}` : download.stage === "preparing" ? "Preparing translation…" : "Translating on this device…"}</span>{download.stage === "downloading" && <small>{formatBytes(download.downloaded)} of {formatBytes(download.total)}</small>}</div>
        <button aria-label="Cancel translation" onClick={cancel}>Cancel</button>
      </div>}
      {hostFallback && <p className="translation-location" role="status">Using host translation because this device could not finish.</p>}
      {policy.isError && source && <p role="alert">Could not load translation preferences. <button onClick={() => void policy.refetch()}>Retry</button></p>}
      <ErrorNotice error={error || progress.error || translated.error || (allowImages && imageQuery.error ? new Error("Images could not be loaded. You can try again.") : undefined)} />

      {current && ["pending", "running"].includes(current.status) && (
        <p role="status">
          {current.status === "pending"
            ? "Preparing translation model…"
            : "Translating…"}
        </p>
      )}
      {current?.status === "failed" && (
        <p role="alert">Translation failed. Try again.</p>
      )}
      {show && result ? (
        <MailReader
          html={result.html}
          text={result.text}
          dark={dark}
          title="Translated message"
          images={images}
        />
      ) : (
        <MailReader html={html} text={text} dark={dark} images={images} />
      )}
      {failedImages > 0 && Object.keys(images || {}).length === 0 && <details className="image-status"><summary>{failedImages} {failedImages === 1 ? "image" : "images"} could not be loaded.</summary><ul>{failures.map(([url, reason]) => <li key={url}>Image {imageSources.indexOf(url) + 1}: {reason}</li>)}</ul></details>}
    </div>
  );
}
