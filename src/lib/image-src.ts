/**
 * A diagram's source as the webview can load it. Imported diagrams are files
 * on disk (`C:\...\fig.png`, `/Users/.../fig.png`); the webview reads those
 * only through Tauri's asset protocol, so they go through `convertFileSrc`.
 * URLs (`asset:`, `data:`, `http:`) are used as they are.
 */
export function isLocalPath(src: string): boolean {
  // "/Users/..." is a file; "//host/x.png" is a URL without its scheme
  return /^[a-zA-Z]:[\\/]/.test(src) || (src.startsWith("/") && !src.startsWith("//")) || src.startsWith("\\\\");
}

export function resolveImageSrc(src: string, convert: (path: string) => string): string {
  return isLocalPath(src) ? convert(src) : src;
}
