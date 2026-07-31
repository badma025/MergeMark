export type TauriCommandError = {
  code?: string;
  message?: string;
  hint?: string;
  [key: string]: unknown;
};

export function parseTauriError(error: unknown): string {
  if (typeof error === "object" && error !== null) {
    const value = error as TauriCommandError;
    return [value.message, value.code, value.hint]
      .filter((part): part is string => typeof part === "string" && part.trim().length > 0)
      .join(" ");
  }

  if (typeof error === "string") {
    try {
      return parseTauriError(JSON.parse(error) as TauriCommandError);
    } catch {
      return error;
    }
  }

  return String(error);
}
