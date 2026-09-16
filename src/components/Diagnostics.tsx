import { useState } from "react";
import { ipcDiagnostics } from "../lib/ipc";

/**
 * What happened on the last attempt to send pixels to Rust.
 *
 * Shown under an error because the failures that matter here only appear in an
 * installed build, which has no developer console: whether a body crossed as
 * bytes or as JSON is decided inside Tauri's injected internals, and the only
 * way to find out is to send something and report how it arrived.
 */
export function Diagnostics() {
  const [copied, setCopied] = useState(false);
  const attempts = ipcDiagnostics();

  const report = [
    `WebView: ${navigator.userAgent}`,
    `Platforma: ${navigator.platform}`,
    `Pokušaji slanja piksela:`,
    ...(attempts.length
      ? attempts.map((a) => `  ${a.form}: ${a.ok ? "OK" : "greška"} — ${a.detail}`)
      : ["  (nijedan)"]),
  ].join("\n");

  const copy = () => {
    void navigator.clipboard.writeText(report).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      },
      () => {},
    );
  };

  return (
    <details className="diagnostics">
      <summary>Dijagnostika</summary>
      <pre>{report}</pre>
      <button type="button" onClick={copy}>
        {copied ? "Kopirano" : "Kopiraj"}
      </button>
    </details>
  );
}
