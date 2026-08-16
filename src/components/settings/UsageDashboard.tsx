import { useState, useEffect, useCallback, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { 
  DollarSign, 
  Zap, 
  FileText, 
  RefreshCw, 
  Layers, 
  CheckCircle2, 
  AlertCircle, 
  Calculator,
  ExternalLink,
  Coins,
  Trash2
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

export interface OpenRouterKeyInfo {
  label: string;
  usageUsd: number;
  limitUsd: number | null;
  isFreeTier: boolean;
  rateLimitRequests: number;
  rateLimitInterval: string;
}

export interface ImportCostRecord {
  id: string;
  paperName: string;
  modelName: string;
  paperType: string;
  questionsCount: number;
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  costUsd: number;
  durationMs: number;
  createdAt: number;
}

const MODEL_PRICING_ESTIMATES: Record<string, { input: number; output: number; name: string }> = {
  "google/gemini-3.7-flash": { input: 0.10, output: 0.40, name: "Gemini 3.7 Flash (Fast & Cheap)" },
  "google/gemini-2.5-flash": { input: 0.10, output: 0.40, name: "Gemini 2.5 Flash" },
  "google/gemini-2.0-flash-001": { input: 0.10, output: 0.40, name: "Gemini 2.0 Flash" },
  "deepseek/deepseek-chat": { input: 0.14, output: 0.28, name: "DeepSeek V3" },
  "openai/gpt-4o-mini": { input: 0.15, output: 0.60, name: "GPT-4o mini" },
  "anthropic/claude-3.5-haiku": { input: 0.80, output: 4.00, name: "Claude 3.5 Haiku" },
  "openai/gpt-4o": { input: 2.50, output: 10.00, name: "GPT-4o (Flagship)" },
  "anthropic/claude-3.5-sonnet": { input: 3.00, output: 15.00, name: "Claude 3.5 Sonnet (Premium)" },
  "anthropic/claude-3.7-sonnet": { input: 3.00, output: 15.00, name: "Claude 3.7 Sonnet (Reasoning)" },
};

export function getRecordCost(record: ImportCostRecord): number {
  if (record.costUsd && record.costUsd > 0) {
    return record.costUsd;
  }
  const m = (record.modelName || "").toLowerCase();
  let inRate = 0.10;
  let outRate = 0.40;
  if (m.includes("deepseek")) {
    inRate = 0.14; outRate = 0.28;
  } else if (m.includes("gpt-4o-mini")) {
    inRate = 0.15; outRate = 0.60;
  } else if (m.includes("haiku")) {
    inRate = 0.80; outRate = 4.00;
  } else if (m.includes("gpt-4o")) {
    inRate = 2.50; outRate = 10.00;
  } else if (m.includes("sonnet")) {
    inRate = 3.00; outRate = 15.00;
  }
  // Reasoning models (3.7 Flash, o1, o3) generate internal reasoning tokens
  // that are billed but not counted in the visible prompt/completion split.
  // Apply a multiplier to approximate the real billed cost.
  const isReasoning = m.includes("3.7") || m.includes("o1") || m.includes("o3");
  const reasoningMultiplier = isReasoning ? 3.0 : 1.0;
  const promptTokens = record.promptTokens > 0 ? record.promptTokens : Math.round(record.totalTokens * 0.8);
  const compTokens = record.completionTokens > 0 ? record.completionTokens : Math.round(record.totalTokens * 0.2);
  const inCost = (promptTokens / 1_000_000) * inRate;
  const outCost = (compTokens / 1_000_000) * outRate;
  return (inCost + outCost) * reasoningMultiplier;
}

export function UsageDashboard({ apiKey }: { apiKey?: string }) {
  const [keyInfo, setKeyInfo] = useState<OpenRouterKeyInfo | null>(null);
  const [history, setHistory] = useState<ImportCostRecord[]>([]);
  const [loadingKey, setLoadingKey] = useState(false);
  const [loadingHistory, setLoadingHistory] = useState(false);
  const [keyError, setKeyError] = useState<string | null>(null);
  const [calcPages, setCalcPages] = useState(25);

  const fetchLiveUsage = useCallback(async () => {
    setLoadingKey(true);
    setKeyError(null);
    try {
      const info = await invoke<OpenRouterKeyInfo>("get_openrouter_usage", {
        apiKey: apiKey || null,
      });
      setKeyInfo(info);
    } catch (err: any) {
      const msg = String(err);
      setKeyError(msg);
    } finally {
      setLoadingKey(false);
    }
  }, [apiKey]);

  const fetchHistory = useCallback(async () => {
    setLoadingHistory(true);
    try {
      const records = await invoke<ImportCostRecord[]>("get_import_cost_history");
      setHistory(records || []);
    } catch (err) {
      console.error("Failed to fetch import history:", err);
    } finally {
      setLoadingHistory(false);
    }
  }, []);

  const handleDeleteRecord = async (id: string, paperName: string) => {
    try {
      await invoke("delete_import_cost_log", { id });
      setHistory((prev) => prev.filter((r) => r.id !== id));
      toast.success(`Removed import record for "${paperName}"`);
    } catch (err) {
      toast.error(`Failed to delete record: ${String(err)}`);
    }
  };

  const handleClearAllHistory = async () => {
    if (!window.confirm("Are you sure you want to clear all historical import records? This cannot be undone.")) {
      return;
    }
    try {
      await invoke("clear_import_cost_history");
      setHistory([]);
      toast.success("All import history logs cleared.");
    } catch (err) {
      toast.error(`Failed to clear history: ${String(err)}`);
    }
  };

  useEffect(() => {
    fetchLiveUsage();
    fetchHistory();
  }, [fetchLiveUsage, fetchHistory]);

  const totalHistoricalQuestions = useMemo(() => history.reduce((acc, h) => acc + (h.questionsCount || 0), 0), [history]);
  const totalHistoricalTokens = useMemo(() => history.reduce((acc, h) => acc + (h.totalTokens || 0), 0), [history]);
  const totalHistoricalCost = useMemo(() => history.reduce((acc, h) => acc + getRecordCost(h), 0), [history]);

  return (
    <div className="flex flex-col gap-6 w-full">
      {/* ── Top Summary & Live OpenRouter Spend Card ── */}
      <div className="rounded-xl border border-border bg-card p-5 shadow-xs flex flex-col gap-4">
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/60 pb-3.5">
          <div className="flex items-center gap-2.5">
            <div className="size-8 rounded-lg bg-primary/10 border border-primary/20 flex items-center justify-center text-primary">
              <DollarSign className="size-4" />
            </div>
            <div>
              <h3 className="text-sm font-semibold text-foreground flex items-center gap-2">
                <span>OpenRouter Live Spend & Key Audit</span>
                {keyInfo && !keyError && (
                  <Badge variant="outline" className="text-[10px] font-mono text-emerald-500 bg-emerald-500/10 border-emerald-500/30">
                    Active
                  </Badge>
                )}
              </h3>
              <p className="text-xs text-muted-foreground">
                Live billing data queried directly from the OpenRouter API for this key.
              </p>
            </div>
          </div>

          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              fetchLiveUsage();
              fetchHistory();
              toast.success("Usage stats synced with OpenRouter");
            }}
            disabled={loadingKey}
            className="h-8 gap-1.5 text-xs font-medium"
          >
            <RefreshCw className={cn("size-3.5", loadingKey && "animate-spin text-primary")} />
            <span>Sync with OpenRouter</span>
          </Button>
        </div>

        {/* Live Key Stats Grid */}
        {keyError ? (
          <div className="flex items-start gap-2.5 p-3.5 rounded-lg bg-amber-500/10 border border-amber-500/20 text-amber-700 dark:text-amber-400 text-xs">
            <AlertCircle className="size-4 shrink-0 mt-0.5" />
            <div>
              <span className="font-semibold block">OpenRouter Live API Status:</span>
              <span className="text-[11px] opacity-90">{keyError}</span>
            </div>
          </div>
        ) : (
          <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-4 gap-3">
            {/* Total USD Spent */}
            <div className="p-3.5 rounded-lg bg-muted/40 border border-border/60 flex flex-col justify-between">
              <div className="flex items-center justify-between text-muted-foreground text-xs">
                <span>Total Live Spend</span>
                <DollarSign className="size-3.5 text-primary" />
              </div>
              <div className="mt-2">
                <span className="text-2xl font-bold font-mono text-foreground tracking-tight">
                  ${Number(keyInfo?.usageUsd ?? (keyInfo as any)?.usage_usd ?? 0).toFixed(4)}
                </span>
                <span className="text-[10px] text-muted-foreground block mt-0.5">
                  USD across all generations
                </span>
              </div>
            </div>

            {/* Credit Limit / Remaining */}
            <div className="p-3.5 rounded-lg bg-muted/40 border border-border/60 flex flex-col justify-between">
              <div className="flex items-center justify-between text-muted-foreground text-xs">
                <span>Credit Limit</span>
                <CheckCircle2 className="size-3.5 text-emerald-500" />
              </div>
              <div className="mt-2">
                <span className="text-xl font-bold font-mono text-foreground">
                  {(keyInfo?.limitUsd ?? (keyInfo as any)?.limit_usd) != null 
                    ? `$${Number(keyInfo?.limitUsd ?? (keyInfo as any)?.limit_usd).toFixed(2)}` 
                    : "No Limit"}
                </span>
                {(keyInfo?.limitUsd ?? (keyInfo as any)?.limit_usd) != null && (
                  <div className="w-full bg-muted/80 h-1.5 rounded-full overflow-hidden mt-1.5">
                    <div 
                      className="bg-primary h-full rounded-full transition-all"
                      style={{ 
                        width: `${Math.min((Number(keyInfo?.usageUsd ?? (keyInfo as any)?.usage_usd ?? 0) / Number(keyInfo?.limitUsd ?? (keyInfo as any)?.limit_usd ?? 1)) * 100, 100)}%` 
                      }}
                    />
                  </div>
                )}
                <span className="text-[10px] text-muted-foreground block mt-0.5">
                  {(keyInfo?.limitUsd ?? (keyInfo as any)?.limit_usd) != null 
                    ? `${((Number(keyInfo?.usageUsd ?? (keyInfo as any)?.usage_usd ?? 0) / Number(keyInfo?.limitUsd ?? (keyInfo as any)?.limit_usd ?? 1)) * 100).toFixed(1)}% of limit used` 
                    : "Pay-as-you-go balance"}
                </span>
              </div>
            </div>

            {/* Key Label */}
            <div className="p-3.5 rounded-lg bg-muted/40 border border-border/60 flex flex-col justify-between">
              <div className="flex items-center justify-between text-muted-foreground text-xs">
                <span>Key Label</span>
                <FileText className="size-3.5 text-blue-500" />
              </div>
              <div className="mt-2">
                <span className="text-sm font-bold text-foreground truncate block" title={keyInfo?.label}>
                  {keyInfo?.label || "Primary Key"}
                </span>
                <span className="text-[10px] text-muted-foreground block mt-0.5">
                  {keyInfo?.isFreeTier ? "Free Tier Key" : "Paid Account Key"}
                </span>
              </div>
            </div>

            {/* Rate Limit */}
            <div className="p-3.5 rounded-lg bg-muted/40 border border-border/60 flex flex-col justify-between">
              <div className="flex items-center justify-between text-muted-foreground text-xs">
                <span>Rate Limits</span>
                <Zap className="size-3.5 text-amber-500" />
              </div>
              <div className="mt-2">
                <span className="text-sm font-bold font-mono text-foreground">
                  {keyInfo?.rateLimitRequests ? `${keyInfo.rateLimitRequests} req / ${keyInfo.rateLimitInterval}` : "Standard"}
                </span>
                <span className="text-[10px] text-muted-foreground block mt-0.5">
                  Concurrent throughput
                </span>
              </div>
            </div>
          </div>
        )}

        <div className="flex items-center justify-between text-[11px] text-muted-foreground pt-2 border-t border-border/40">
          <span>View complete generation activity in OpenRouter dashboard:</span>
          <a
            href="https://openrouter.ai/activity"
            target="_blank"
            rel="noreferrer"
            className="flex items-center gap-1 text-primary hover:underline font-medium"
          >
            <span>openrouter.ai/activity</span>
            <ExternalLink className="size-3" />
          </a>
        </div>
      </div>

      {/* ── Cost Comparison Calculator for Exam Papers ── */}
      <div className="rounded-xl border border-border bg-card p-5 shadow-xs flex flex-col gap-4">
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/60 pb-3">
          <div className="flex items-center gap-2">
            <Calculator className="size-4 text-primary" />
            <h3 className="text-sm font-semibold text-foreground">Exam Paper Ingestion Cost Estimator</h3>
          </div>
          <div className="flex items-center gap-2 text-xs">
            <span className="text-muted-foreground">Sample Exam Length:</span>
            <div className="flex items-center gap-1">
              {[10, 25, 40].map((pages) => (
                <Button
                  key={pages}
                  variant={calcPages === pages ? "default" : "outline"}
                  size="sm"
                  onClick={() => setCalcPages(pages)}
                  className="h-6 px-2 text-xs"
                >
                  {pages} pages
                </Button>
              ))}
            </div>
          </div>
        </div>

        <p className="text-xs text-muted-foreground">
          Calculated for a typical {calcPages}-page exam paper (~{Math.round(calcPages * 0.8)} questions, ~{(calcPages * 1.4).toFixed(0)}k input tokens with high-resolution vision tiles):
        </p>

        <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-4 gap-3">
          {Object.entries(MODEL_PRICING_ESTIMATES).slice(0, 4).map(([id, pricing]) => {
            const inputTokens = calcPages * 1400;
            const outputTokens = Math.round(calcPages * 0.8) * 350;
            const cost = (inputTokens / 1_000_000) * pricing.input + (outputTokens / 1_000_000) * pricing.output;
            const isGemini = id.includes("gemini-3.7-flash") || id.includes("gemini-2.5-flash");

            return (
              <div 
                key={id} 
                className={cn(
                  "p-3 rounded-lg border flex flex-col justify-between relative transition-all",
                  isGemini ? "bg-primary/5 border-primary/40 ring-1 ring-primary/20" : "bg-muted/30 border-border/60"
                )}
              >
                {isGemini && (
                  <Badge variant="default" className="absolute -top-2 right-2 text-[9px] px-1.5 py-0 h-4">
                    Fastest
                  </Badge>
                )}
                <div>
                  <span className="text-xs font-semibold text-foreground block truncate" title={pricing.name}>
                    {pricing.name}
                  </span>
                  <span className="text-[10px] font-mono text-muted-foreground block mt-0.5">
                    ${pricing.input.toFixed(2)} in / ${pricing.output.toFixed(2)} out
                  </span>
                </div>
                <div className="mt-2.5 pt-2 border-t border-border/40 flex items-baseline justify-between">
                  <span className="text-[11px] text-muted-foreground">Est. per paper:</span>
                  <span className="text-sm font-bold font-mono text-foreground">
                    ${cost.toFixed(3)}
                  </span>
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {/* ── Historical Import Audit Table with Cost Breakdown ── */}
      <div className="rounded-xl border border-border bg-card p-5 shadow-xs flex flex-col gap-4">
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/60 pb-3">
          <div className="flex items-center gap-2">
            <Layers className="size-4 text-primary" />
            <div>
              <h3 className="text-sm font-semibold text-foreground">Historical Import Log & Spend Audit</h3>
              <p className="text-xs text-muted-foreground">
                Recorded past papers processed in this MergeMark repository. Automatically pruned when papers are deleted.
              </p>
            </div>
          </div>
          <div className="flex items-center gap-2.5 flex-wrap">
            <div className="flex items-center gap-2 text-xs font-mono text-muted-foreground">
              <span>{history.length} runs</span>
              <span>·</span>
              <span>{totalHistoricalQuestions} questions</span>
              {totalHistoricalTokens > 0 && (
                <>
                  <span>·</span>
                  <span>{(totalHistoricalTokens / 1_000).toFixed(1)}k tokens</span>
                </>
              )}
              <span>·</span>
              <Badge variant="secondary" className="font-mono text-xs font-bold text-primary bg-primary/10 border-primary/20 gap-1 px-2">
                <Coins className="size-3" />
                <span>${totalHistoricalCost.toFixed(4)} USD</span>
              </Badge>
            </div>

            {history.length > 0 && (
              <Button
                variant="ghost"
                size="sm"
                className="h-7 text-xs text-muted-foreground hover:text-destructive hover:bg-destructive/10 px-2 gap-1.5"
                onClick={handleClearAllHistory}
                title="Clear all import logs"
              >
                <Trash2 className="size-3.5" />
                <span>Clear History</span>
              </Button>
            )}
          </div>
        </div>

        {loadingHistory ? (
          <div className="text-center p-8 border border-dashed border-border/70 rounded-lg text-muted-foreground text-xs flex items-center justify-center gap-2">
            <RefreshCw className="size-4 animate-spin text-primary" />
            <span>Loading import audit records...</span>
          </div>
        ) : history.length === 0 ? (
          <div className="text-center p-8 border border-dashed border-border/70 rounded-lg text-muted-foreground text-xs flex flex-col items-center gap-1.5">
            <FileText className="size-6 opacity-30" />
            <span>No import history recorded.</span>
            <span className="text-[11px] opacity-75">When you import exam papers via the Ingestion tab, their run metrics and costs will appear here.</span>
          </div>
        ) : (
          <div className="overflow-x-auto rounded-lg border border-border/60 max-w-full">
            <table className="w-full text-xs text-left border-collapse">
              <thead className="bg-muted/60 text-muted-foreground border-b border-border/60">
                <tr>
                  <th className="p-2.5 px-3 font-semibold">Paper Name</th>
                  <th className="p-2.5 px-3 font-semibold">Model</th>
                  <th className="p-2.5 px-3 font-semibold text-center">Questions</th>
                  <th className="p-2.5 px-3 font-semibold text-right">Tokens</th>
                  <th className="p-2.5 px-3 font-semibold text-right text-foreground font-bold">Cost (USD)</th>
                  <th className="p-2.5 px-3 font-semibold text-right">Duration</th>
                  <th className="p-2.5 px-3 font-semibold text-right">Date</th>
                  <th className="p-2.5 px-2 font-semibold text-center w-8"></th>
                </tr>
              </thead>
              <tbody className="divide-y divide-border/40">
                {history.map((record) => {
                  const cost = getRecordCost(record);
                  return (
                    <tr key={record.id} className="hover:bg-muted/20 transition-colors font-mono group">
                      <td className="p-2.5 px-3 font-sans font-medium text-foreground truncate max-w-[200px]" title={record.paperName}>
                        {record.paperName}
                      </td>
                      <td className="p-2.5 px-3 text-muted-foreground truncate max-w-[160px]" title={record.modelName}>
                        {record.modelName.replace("google/", "").replace("anthropic/", "").replace("openai/", "")}
                      </td>
                      <td className="p-2.5 px-3 text-center text-foreground font-semibold">
                        {record.questionsCount}
                      </td>
                      <td className="p-2.5 px-3 text-right text-muted-foreground">
                        {record.totalTokens > 0 ? `${(record.totalTokens / 1_000).toFixed(1)}k` : "—"}
                      </td>
                      <td className="p-2.5 px-3 text-right font-bold text-foreground">
                        {cost > 0 ? `$${cost.toFixed(4)}` : "<$0.001"}
                      </td>
                      <td className="p-2.5 px-3 text-right text-muted-foreground">
                        {record.durationMs > 0 ? `${(record.durationMs / 1000).toFixed(1)}s` : "—"}
                      </td>
                      <td className="p-2.5 px-3 text-right text-muted-foreground font-sans text-[11px]">
                        {new Date(record.createdAt * 1000).toLocaleDateString()}
                      </td>
                      <td className="p-2.5 px-2 text-center">
                        <button
                          type="button"
                          onClick={() => handleDeleteRecord(record.id, record.paperName)}
                          className="text-muted-foreground/50 hover:text-destructive transition-colors p-1 rounded hover:bg-destructive/10 opacity-0 group-hover:opacity-100 focus:opacity-100"
                          title="Delete this import record"
                        >
                          <Trash2 className="size-3.5" />
                        </button>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>
    </div>
  );
}
