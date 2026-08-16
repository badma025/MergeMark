import { useState, useEffect } from "react";
import { 
  Settings as SettingsIcon, 
  Key, 
  RefreshCw, 
  AlertCircle, 
  Trash2, 
  Archive, 
  Download, 
  Upload, 
  Sparkles, 
  DollarSign, 
  FolderTree, 
  ShieldAlert,
  Server,
  Eye,
  EyeOff
} from "lucide-react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { TaxonomyManager } from "./TaxonomyManager";
import { SplashScreen } from "@/components/common/SplashScreen";
import { UsageDashboard } from "./UsageDashboard";
import { cn } from "@/lib/utils";

interface ExportReport {
  questions: number;
  images: number;
  missingImages: number;
  path: string;
}
interface BackupPreview {
  formatVersion: number;
  appVersion: string;
  exportedAt: number;
  questionCount: number;
  imageCount: number;
}
interface ImportSummary {
  added: number;
  updated: number;
  imagesCopied: number;
  replaced: boolean;
}
interface PendingImport {
  path: string;
  preview: BackupPreview;
}

export function Settings() {
  const [activeTab, setActiveTab] = useState<"api_usage" | "taxonomy" | "backup_danger">("api_usage");

  const [apiKey, setApiKey] = useState("");
  const [showApiKey, setShowApiKey] = useState(false);
  const [baseUrl, setBaseUrl] = useState("https://openrouter.ai/api/v1");
  const [modelName, setModelName] = useState("google/gemini-2.5-flash");
  const [availableModels, setAvailableModels] = useState<string[]>([]);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [modelFetchError, setModelFetchError] = useState("");
  const [confirmingClear, setConfirmingClear] = useState(false);
  const [clearInput, setClearInput] = useState("");
  const [clearing, setClearing] = useState(false);

  const REQUIRED_CLEAR_PHRASE = "I understand that this will permanently delete all my questions";
  const REQUIRED_REPLACE_PHRASE = "I understand that this will replace my entire library";

  const [backupBusy, setBackupBusy] = useState<"export" | "import" | null>(null);
  const [pendingImport, setPendingImport] = useState<PendingImport | null>(null);
  const [importMode, setImportMode] = useState<"merge" | "replace">("merge");
  const [replaceInput, setReplaceInput] = useState("");
  const [showSplashPreview, setShowSplashPreview] = useState(false);

  useEffect(() => {
    const savedKey = localStorage.getItem("mergemark_openai_key");
    if (savedKey) setApiKey(savedKey);

    const savedBaseUrl = localStorage.getItem("mergemark_openai_base_url");
    if (savedBaseUrl) setBaseUrl(savedBaseUrl);

    const savedModel = localStorage.getItem("mergemark_openai_model");
    if (savedModel) setModelName(savedModel);

    // Sync to backend for billing logic
    invoke("set_byok_key", { 
      apiKey: savedKey || null, 
      baseUrl: savedBaseUrl || null 
    }).catch(console.error);
  }, []);

  function handleKeyChange(e: React.ChangeEvent<HTMLInputElement>) {
    const newKey = e.target.value;
    setApiKey(newKey);
    localStorage.setItem("mergemark_openai_key", newKey);
    invoke("set_byok_key", { apiKey: newKey || null, baseUrl: baseUrl || null }).catch(console.error);
  }

  function handleBaseUrlChange(e: React.ChangeEvent<HTMLInputElement>) {
    const newUrl = e.target.value;
    setBaseUrl(newUrl);
    localStorage.setItem("mergemark_openai_base_url", newUrl);
    invoke("set_byok_key", { apiKey: apiKey || null, baseUrl: newUrl || null }).catch(console.error);
  }

  function handleModelChange(e: React.ChangeEvent<HTMLInputElement>) {
    const newModel = e.target.value;
    setModelName(newModel);
    localStorage.setItem("mergemark_openai_model", newModel);
  }

  async function handleFetchModels() {
    setFetchingModels(true);
    setModelFetchError("");
    try {
      const models = await invoke<string[]>("fetch_models", { baseUrl, apiKey });
      setAvailableModels(models);
    } catch (err: any) {
      setModelFetchError(err.toString());
    } finally {
      setFetchingModels(false);
    }
  }

  async function handleExportBackup() {
    setBackupBusy("export");
    try {
      const path = await save({
        defaultPath: `mergemark-backup-${new Date().toISOString().slice(0, 10)}.zip`,
        filters: [{ name: "MergeMark Backup", extensions: ["zip"] }],
      });
      if (!path) return;
      const report = await invoke<ExportReport>("export_backup", { destPath: path });
      if (report.missingImages > 0) {
        toast.warning(`Backup saved — ${report.questions} questions, ${report.images} images`, {
          description: `${report.missingImages} diagram reference(s) pointed to files that no longer exist on disk and were left out.`,
        });
      } else {
        toast.success(`Backup saved — ${report.questions} questions, ${report.images} images`);
      }
    } catch (err) {
      toast.error("Backup failed", { description: String(err) });
    } finally {
      setBackupBusy(null);
    }
  }

  async function handlePickBackup() {
    setBackupBusy("import");
    try {
      const path = await open({
        multiple: false,
        filters: [{ name: "MergeMark Backup", extensions: ["zip"] }],
      });
      if (!path || typeof path !== "string") return;
      const preview = await invoke<BackupPreview>("preview_backup", { srcPath: path });
      setPendingImport({ path, preview });
      setImportMode("merge");
      setReplaceInput("");
    } catch (err) {
      toast.error("Could not read that backup", { description: String(err) });
    } finally {
      setBackupBusy(null);
    }
  }

  async function handleImportBackup() {
    if (!pendingImport) return;
    if (importMode === "replace" && replaceInput !== REQUIRED_REPLACE_PHRASE) return;
    setBackupBusy("import");
    try {
      const summary = await invoke<ImportSummary>("import_backup", {
        srcPath: pendingImport.path,
        mode: importMode,
      });
      const modeText = summary.replaced ? "Library replaced" : "Library updated";
      toast.success(`${modeText} from backup`, {
        description: `Added ${summary.added} new questions, updated ${summary.updated}, copied ${summary.imagesCopied} images.`,
      });
      setPendingImport(null);
      setReplaceInput("");
    } catch (err) {
      toast.error("Restore failed", { description: String(err) });
    } finally {
      setBackupBusy(null);
    }
  }

  async function handleClearRepository() {
    if (clearInput !== REQUIRED_CLEAR_PHRASE) return;
    setClearing(true);
    try {
      await invoke("delete_all_questions");
      toast.success("Question repository cleared");
      setConfirmingClear(false);
      setClearInput("");
    } catch (err) {
      toast.error("Failed to clear repository", { description: String(err) });
    } finally {
      setClearing(false);
    }
  }

  return (
    <section
      className="flex flex-1 flex-col items-center justify-start h-full min-h-0 px-4 sm:px-8 py-6 sm:py-8 bg-background overflow-y-auto"
      aria-label="Settings"
    >
      {/* ── Settings Header ── */}
      <div className="w-full max-w-4xl flex flex-col items-center mb-6 text-center space-y-2 select-none">
        <div className="flex items-center gap-2.5">
          <div className="size-8 rounded-xl bg-primary/10 border border-primary/20 flex items-center justify-center text-primary shadow-xs">
            <SettingsIcon className="size-4.5" />
          </div>
          <h1 className="text-xl font-bold tracking-tight text-foreground">
            Settings &amp; Configuration
          </h1>
        </div>
        <p className="text-xs text-muted-foreground max-w-md">
          Configure API endpoints, monitor real-time OpenRouter spending, manage curriculum taxonomies, and manage database backups.
        </p>

        {/* ── Segmented Tab Switcher ── */}
        <div className="inline-flex items-center p-1 bg-muted/60 dark:bg-muted/40 border border-border/80 rounded-xl shadow-xs mt-3">
          <button
            type="button"
            onClick={() => setActiveTab("api_usage")}
            className={cn(
              "flex items-center gap-2 px-4 py-1.5 rounded-lg text-xs font-semibold transition-all select-none",
              activeTab === "api_usage" 
                ? "bg-card text-foreground shadow-xs border border-border/80" 
                : "text-muted-foreground hover:text-foreground"
            )}
          >
            <DollarSign className="size-3.5 text-primary" />
            <span>API &amp; Spend</span>
          </button>

          <button
            type="button"
            onClick={() => setActiveTab("taxonomy")}
            className={cn(
              "flex items-center gap-2 px-4 py-1.5 rounded-lg text-xs font-semibold transition-all select-none",
              activeTab === "taxonomy" 
                ? "bg-card text-foreground shadow-xs border border-border/80" 
                : "text-muted-foreground hover:text-foreground"
            )}
          >
            <FolderTree className="size-3.5 text-blue-500" />
            <span>Curriculum Taxonomy</span>
          </button>

          <button
            type="button"
            onClick={() => setActiveTab("backup_danger")}
            className={cn(
              "flex items-center gap-2 px-4 py-1.5 rounded-lg text-xs font-semibold transition-all select-none",
              activeTab === "backup_danger" 
                ? "bg-card text-foreground shadow-xs border border-border/80" 
                : "text-muted-foreground hover:text-foreground"
            )}
          >
            <Archive className="size-3.5 text-amber-500" />
            <span>Backup &amp; Maintenance</span>
          </button>
        </div>
      </div>

      {/* ── Tab Content Container ── */}
      <div className="w-full max-w-4xl flex flex-col gap-6 mb-12">
        {/* ════════════════ TAB 1: API & USAGE SPEND ════════════════ */}
        {activeTab === "api_usage" && (
          <div className="flex flex-col gap-6 animate-in fade-in-50 duration-150">
            {/* API Configuration Card */}
            <div className="flex flex-col gap-5 rounded-xl border border-border/70 bg-card p-6 shadow-xs">
              <div className="border-b border-border/60 pb-3">
                <h2 className="text-sm font-bold flex items-center gap-2 text-foreground">
                  <Key className="size-4 text-primary" />
                  API &amp; Model Credentials
                </h2>
                <p className="text-xs text-muted-foreground mt-0.5">
                  MergeMark connects directly to OpenRouter or your custom OpenAI-compatible endpoint using your key.
                </p>
              </div>

              <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                {/* Base URL */}
                <div className="flex flex-col gap-1.5">
                  <label htmlFor="base-url" className="text-xs font-semibold flex items-center gap-1.5 text-foreground">
                    <Server className="size-3.5 text-muted-foreground" />
                    <span>Base URL</span>
                  </label>
                  <Input
                    id="base-url"
                    type="text"
                    placeholder="https://openrouter.ai/api/v1"
                    value={baseUrl}
                    onChange={handleBaseUrlChange}
                    className="font-mono text-xs bg-muted/20"
                  />
                  <p className="text-[11px] text-muted-foreground">
                    Default: <code className="bg-muted px-1 py-0.5 rounded text-[10px]">https://openrouter.ai/api/v1</code>
                  </p>
                </div>

                {/* Model Identifier */}
                <div className="flex flex-col gap-1.5">
                  <div className="flex items-center justify-between">
                    <label htmlFor="model-name" className="text-xs font-semibold text-foreground">
                      <span>Model Identifier</span>
                    </label>
                    <Button 
                      variant="outline" 
                      size="sm" 
                      className="h-6 text-[11px] px-2 gap-1" 
                      onClick={handleFetchModels}
                      disabled={fetchingModels}
                    >
                      <RefreshCw className={cn("size-3", fetchingModels && "animate-spin text-primary")} />
                      <span>Fetch</span>
                    </Button>
                  </div>
                  <div className="relative">
                    <Input
                      id="model-name"
                      type="text"
                      list="models-list"
                      placeholder="google/gemini-2.5-flash"
                      value={modelName}
                      onChange={handleModelChange}
                      className="font-mono text-xs w-full bg-muted/20"
                    />
                    {availableModels.length > 0 && (
                      <datalist id="models-list">
                        {availableModels.map(m => <option key={m} value={m} />)}
                      </datalist>
                    )}
                  </div>
                  {modelFetchError ? (
                    <p className="text-[11px] text-destructive flex items-center gap-1">
                      <AlertCircle className="size-3" /> {modelFetchError}
                    </p>
                  ) : (
                    <p className="text-[11px] text-muted-foreground">
                      Recommended: <code className="bg-muted px-1 py-0.5 rounded text-[10px]">google/gemini-2.5-flash</code>
                    </p>
                  )}
                </div>
              </div>

              {/* API Key */}
              <div className="flex flex-col gap-1.5 pt-3 border-t border-border/40">
                <label htmlFor="openai-api-key" className="text-xs font-semibold flex items-center gap-1.5 text-foreground">
                  <Key className="size-3.5 text-primary" />
                  <span>OpenRouter / OpenAI API Key</span>
                </label>
                <div className="relative">
                  <Input
                    id="openai-api-key"
                    type={showApiKey ? "text" : "password"}
                    placeholder="sk-or-v1-..."
                    value={apiKey}
                    onChange={handleKeyChange}
                    className="font-mono text-xs pr-9 bg-muted/20"
                  />
                  <button
                    type="button"
                    onClick={() => setShowApiKey(!showApiKey)}
                    className="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                    title={showApiKey ? "Hide key" : "Show key"}
                  >
                    {showApiKey ? <EyeOff className="size-3.5" /> : <Eye className="size-3.5" />}
                  </button>
                </div>
                <p className="text-[11px] text-muted-foreground">
                  Stored securely on this local device only. Providing an OpenRouter key enables live account spend metrics.
                </p>
              </div>
            </div>

            {/* Live OpenRouter Usage & Historical Spend Dashboard */}
            <UsageDashboard apiKey={apiKey} />
          </div>
        )}

        {/* ════════════════ TAB 2: CURRICULUM TAXONOMY ════════════════ */}
        {activeTab === "taxonomy" && (
          <div className="rounded-xl border border-border/70 bg-card p-6 shadow-xs animate-in fade-in-50 duration-150">
            <TaxonomyManager />
          </div>
        )}

        {/* ════════════════ TAB 3: BACKUP & MAINTENANCE ════════════════ */}
        {activeTab === "backup_danger" && (
          <div className="flex flex-col gap-6 animate-in fade-in-50 duration-150">
            {/* Backup & Restore */}
            <div className="flex flex-col gap-4 rounded-xl border border-border/70 bg-card p-6 shadow-xs">
              <div className="border-b border-border/60 pb-3">
                <h2 className="text-sm font-bold flex items-center gap-2 text-foreground">
                  <Archive className="size-4 text-primary" />
                  Repository Backup &amp; Migration
                </h2>
                <p className="text-xs text-muted-foreground mt-0.5">
                  Export all extracted questions, mark schemes, and cropped diagrams to a compressed zip bundle.
                </p>
              </div>

              {!pendingImport ? (
                <div className="flex flex-wrap gap-2.5 pt-1">
                  <Button
                    variant="outline"
                    className="gap-2 text-xs font-medium"
                    onClick={handleExportBackup}
                    disabled={backupBusy !== null}
                  >
                    <Download className="size-3.5 text-primary" />
                    <span>{backupBusy === "export" ? "Exporting..." : "Export Full Backup"}</span>
                  </Button>
                  <Button
                    variant="outline"
                    className="gap-2 text-xs font-medium"
                    onClick={handlePickBackup}
                    disabled={backupBusy !== null}
                  >
                    <Upload className="size-3.5 text-blue-500" />
                    <span>{backupBusy === "import" ? "Reading..." : "Import Backup File"}</span>
                  </Button>
                </div>
              ) : (
                <div className="flex flex-col gap-3 bg-muted/20 p-4 rounded-xl border border-border/60">
                  <p className="text-xs font-semibold text-foreground">Ready to import backup</p>
                  <p className="text-xs text-muted-foreground">
                    Created on{" "}
                    <span className="font-medium text-foreground">
                      {new Date(pendingImport.preview.exportedAt * 1000).toLocaleString()}
                    </span>{" "}
                    · {pendingImport.preview.questionCount} questions · {pendingImport.preview.imageCount} diagrams
                  </p>

                  <label className="flex items-start gap-2 text-xs cursor-pointer mt-1">
                    <input
                      type="radio"
                      name="import-mode"
                      className="mt-0.5 accent-primary"
                      checked={importMode === "merge"}
                      onChange={() => setImportMode("merge")}
                    />
                    <span>
                      <span className="font-semibold">Merge</span>
                      <span className="block text-[11px] text-muted-foreground">
                        Add new questions and update existing ones. Safe (recommended).
                      </span>
                    </span>
                  </label>

                  <label className="flex items-start gap-2 text-xs cursor-pointer">
                    <input
                      type="radio"
                      name="import-mode"
                      className="mt-0.5 accent-destructive"
                      checked={importMode === "replace"}
                      onChange={() => setImportMode("replace")}
                    />
                    <span>
                      <span className="font-semibold text-destructive">Replace entire library</span>
                      <span className="block text-[11px] text-muted-foreground">
                        Permanently wipe your current repository and replace it with this backup.
                      </span>
                    </span>
                  </label>

                  {importMode === "replace" && (
                    <div className="flex flex-col gap-2 mt-2 p-3 bg-destructive/10 rounded-lg border border-destructive/20 text-xs">
                      <p className="text-destructive font-medium">
                        Type <code className="bg-destructive/20 px-1 py-0.5 rounded text-destructive select-all">{REQUIRED_REPLACE_PHRASE}</code> to confirm:
                      </p>
                      <Input
                        type="text"
                        placeholder="Type the confirmation phrase"
                        value={replaceInput}
                        onChange={(e) => setReplaceInput(e.target.value)}
                        className="text-xs"
                      />
                    </div>
                  )}

                  <div className="flex gap-2 justify-end mt-2 pt-2 border-t border-border/40">
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => {
                        setPendingImport(null);
                        setReplaceInput("");
                      }}
                      disabled={backupBusy !== null}
                      className="text-xs"
                    >
                      Cancel
                    </Button>
                    <Button
                      variant={importMode === "replace" ? "destructive" : "default"}
                      size="sm"
                      onClick={handleImportBackup}
                      disabled={backupBusy !== null || (importMode === "replace" && replaceInput !== REQUIRED_REPLACE_PHRASE)}
                      className="text-xs font-semibold"
                    >
                      {backupBusy === "import" ? "Restoring..." : importMode === "replace" ? "Replace Library" : "Merge Library"}
                    </Button>
                  </div>
                </div>
              )}
            </div>

            {/* Particle Splash Screen Preview */}
            <div className="flex flex-col gap-3 rounded-xl border border-border/70 bg-card p-6 shadow-xs">
              <div className="border-b border-border/60 pb-3">
                <h2 className="text-sm font-bold flex items-center gap-2 text-foreground">
                  <Sparkles className="size-4 text-amber-500" />
                  Intro Screen Preview
                </h2>
                <p className="text-xs text-muted-foreground mt-0.5">
                  Launch the animated startup particle screen.
                </p>
              </div>
              <Button
                variant="outline"
                size="sm"
                className="w-fit text-xs gap-2"
                onClick={() => setShowSplashPreview(true)}
              >
                <Sparkles className="size-3.5 text-amber-500" />
                <span>Launch Splash Screen</span>
              </Button>
            </div>

            {/* Danger Zone: Clear Repository */}
            <div className="flex flex-col gap-4 rounded-xl border border-destructive/40 bg-destructive/5 p-6 shadow-xs">
              <div className="flex items-center gap-2 text-destructive border-b border-destructive/20 pb-3">
                <ShieldAlert className="size-4" />
                <h2 className="text-sm font-bold">Danger Zone</h2>
              </div>
              <p className="text-xs text-muted-foreground">
                Permanently purge all questions, answers, and cached extractions in the SQLite repository. This action cannot be undone.
              </p>

              {!confirmingClear ? (
                <Button 
                  variant="outline" 
                  size="sm" 
                  className="w-fit text-destructive hover:bg-destructive/10 hover:text-destructive border-destructive/30 gap-2 text-xs font-semibold"
                  onClick={() => setConfirmingClear(true)}
                >
                  <Trash2 className="size-3.5" />
                  <span>Clear All Questions</span>
                </Button>
              ) : (
                <div className="flex flex-col gap-3 bg-background p-4 rounded-xl border border-destructive/30 text-xs">
                  <p className="text-xs font-medium text-foreground">
                    Type <code className="bg-muted px-1.5 py-0.5 rounded text-destructive select-all">{REQUIRED_CLEAR_PHRASE}</code> below to confirm:
                  </p>
                  <Input
                    type="text"
                    placeholder="Type the phrase above"
                    value={clearInput}
                    onChange={(e) => setClearInput(e.target.value)}
                    className="text-xs"
                  />
                  <div className="flex gap-2 justify-end mt-1">
                    <Button 
                      variant="ghost" 
                      size="sm"
                      onClick={() => {
                        setConfirmingClear(false);
                        setClearInput("");
                      }}
                      disabled={clearing}
                      className="text-xs"
                    >
                      Cancel
                    </Button>
                    <Button 
                      variant="destructive" 
                      size="sm" 
                      disabled={clearInput !== REQUIRED_CLEAR_PHRASE || clearing}
                      onClick={handleClearRepository}
                      className="text-xs font-semibold"
                    >
                      {clearing ? "Deleting..." : "Permanently Delete All"}
                    </Button>
                  </div>
                </div>
              )}
            </div>
          </div>
        )}
      </div>

      {showSplashPreview && (
        <SplashScreen
          duration={5000}
          onFinish={() => setShowSplashPreview(false)}
        />
      )}
    </section>
  );
}
