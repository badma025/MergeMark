import { useState, useEffect } from "react";
import { Settings as SettingsIcon, Key, RefreshCw, AlertCircle, Trash2, AlertTriangle, Archive, Download, Upload } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { TaxonomyManager } from "./TaxonomyManager";

// Mirrors the serde camelCase report structs in src-tauri/src/backup.rs
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
  const [apiKey, setApiKey] = useState("");
  const [baseUrl, setBaseUrl] = useState("https://openrouter.ai/api/v1/");
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
      if (models.length > 0 && !models.includes(modelName)) {
        // optionally don't auto-set, just let the user pick
      }
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
      if (!path) return; // user cancelled the picker
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
      if (!path || typeof path !== "string") return; // user cancelled the picker
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
      toast.success(
        summary.replaced
          ? `Library restored — ${summary.added} questions, ${summary.imagesCopied} images`
          : `Import complete — ${summary.added} new, ${summary.updated} updated, ${summary.imagesCopied} images restored`
      );
      setPendingImport(null);
    } catch (err) {
      toast.error("Import failed", { description: String(err) });
    } finally {
      setBackupBusy(null);
    }
  }

  async function handleClearRepository() {
    if (clearInput !== REQUIRED_CLEAR_PHRASE) return;
    
    setClearing(true);
    try {
      await invoke("delete_all_questions");
      toast.success("Repository cleared successfully");
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
      className="flex flex-1 flex-col items-center justify-start h-full min-h-0 px-8 py-12 bg-background overflow-y-auto"
      aria-label="Settings"
    >
      <div className="w-full max-w-md flex flex-col items-center mt-4 mb-8 text-center space-y-1 select-none">
        <h1 className="text-2xl font-bold tracking-tight text-foreground flex items-center justify-center gap-2">
          <SettingsIcon className="size-6 text-primary" />
          Settings
        </h1>
        <p className="text-sm text-muted-foreground max-w-md">
          Configure MergeMark preferences and API integrations.
        </p>
      </div>

      <div className="w-full max-w-md flex flex-col gap-6 rounded-2xl border border-border/60 bg-card p-6 shadow-sm mb-12">
        
        {/* Base URL */}
        <div className="flex flex-col gap-3">
          <label htmlFor="base-url" className="text-sm font-semibold flex items-center gap-2">
            Base URL
          </label>
          <Input
            id="base-url"
            type="text"
            placeholder="https://api.openai.com/v1"
            value={baseUrl}
            onChange={handleBaseUrlChange}
            className="font-mono"
            aria-describedby="base-url-description"
          />
          <p id="base-url-description" className="text-xs text-muted-foreground">
            The API endpoint. For Ollama, use <code className="bg-muted px-1 py-0.5 rounded">http://localhost:11434/v1</code>.
          </p>
        </div>

        {/* Model Name */}
        <div className="flex flex-col gap-3">
          <label htmlFor="model-name" className="text-sm font-semibold flex items-center justify-between">
            <span className="flex items-center gap-2">Model Name</span>
            <Button 
              variant="outline" 
              size="sm" 
              className="h-7 text-xs" 
              onClick={handleFetchModels}
              disabled={fetchingModels}
            >
              {fetchingModels ? <RefreshCw className="size-3 mr-1 animate-spin" /> : <RefreshCw className="size-3 mr-1" />}
              Fetch
            </Button>
          </label>
          <div className="relative">
            <Input
              id="model-name"
              type="text"
              list="models-list"
              placeholder="gpt-4o-mini"
              value={modelName}
              onChange={handleModelChange}
              className="font-mono w-full"
              aria-describedby="model-name-description"
            />
            {availableModels.length > 0 && (
              <datalist id="models-list">
                {availableModels.map(m => <option key={m} value={m} />)}
              </datalist>
            )}
          </div>
          {modelFetchError && (
            <p className="text-xs text-destructive flex items-center gap-1">
              <AlertCircle className="size-3" /> {modelFetchError}
            </p>
          )}
          <p id="model-name-description" className="text-xs text-muted-foreground">
            The model to use. You can click Fetch to load available models.
          </p>
        </div>

        {/* API Key */}
        <div className="flex flex-col gap-3">
          <label htmlFor="openai-api-key" className="text-sm font-semibold flex items-center gap-2">
            <Key className="size-4 text-primary" />
            API Key
          </label>
          <Input
            id="openai-api-key"
            type="password"
            placeholder="sk-..."
            value={apiKey}
            onChange={handleKeyChange}
            className="font-mono"
            aria-describedby="openai-api-key-description"
          />
          <p id="openai-api-key-description" className="text-xs text-muted-foreground">
            Required for OpenAI/cloud providers. Leave blank or enter a dummy value for local providers like Ollama.
          </p>
        </div>
      </div>

      {/* ── Backup & Restore ── */}
      <div className="w-full max-w-md flex flex-col gap-4 rounded-2xl border border-border/60 bg-card p-6 shadow-sm mb-12">
        <h2 className="text-sm font-bold flex items-center gap-2 text-foreground">
          <Archive className="size-4 text-primary" />
          Backup &amp; Restore
        </h2>
        <p className="text-sm text-muted-foreground">
          Save your entire question library — including diagrams — to a single file you can
          keep as a backup, move to another computer, or share. Your API key is never included.
        </p>

        {!pendingImport ? (
          <div className="flex gap-2">
            <Button
              variant="outline"
              className="gap-2"
              onClick={handleExportBackup}
              disabled={backupBusy !== null}
            >
              <Download className="size-4" />
              {backupBusy === "export" ? "Exporting..." : "Export backup"}
            </Button>
            <Button
              variant="outline"
              className="gap-2"
              onClick={handlePickBackup}
              disabled={backupBusy !== null}
            >
              <Upload className="size-4" />
              {backupBusy === "import" ? "Reading..." : "Import backup"}
            </Button>
          </div>
        ) : (
          <div className="flex flex-col gap-3 bg-background p-4 rounded-xl border border-border/60">
            <p className="text-sm font-medium text-foreground">
              Ready to import
            </p>
            <p className="text-xs text-muted-foreground">
              Backup from{" "}
              <span className="font-medium text-foreground">
                {new Date(pendingImport.preview.exportedAt * 1000).toLocaleString()}
              </span>{" "}
              · {pendingImport.preview.questionCount} questions ·{" "}
              {pendingImport.preview.imageCount} images · made with app v
              {pendingImport.preview.appVersion}
            </p>

            <label className="flex items-start gap-2 text-sm cursor-pointer mt-1">
              <input
                type="radio"
                name="import-mode"
                className="mt-1 accent-primary"
                checked={importMode === "merge"}
                onChange={() => setImportMode("merge")}
              />
              <span>
                <span className="font-medium">Merge</span>
                <span className="block text-xs text-muted-foreground">
                  Add new questions and update ones that already exist in this backup. Nothing is deleted. Recommended.
                </span>
              </span>
            </label>
            <label className="flex items-start gap-2 text-sm cursor-pointer">
              <input
                type="radio"
                name="import-mode"
                className="mt-1 accent-destructive"
                checked={importMode === "replace"}
                onChange={() => setImportMode("replace")}
              />
              <span>
                <span className="font-medium text-destructive">Replace everything</span>
                <span className="block text-xs text-muted-foreground">
                  Delete the entire current library first, then restore from this backup.
                </span>
              </span>
            </label>

            {importMode === "replace" && (
              <div className="flex flex-col gap-2 mt-1">
                <p className="text-xs font-medium text-foreground">
                  Type{" "}
                  <code className="bg-muted px-1.5 py-0.5 rounded text-destructive select-all">
                    {REQUIRED_REPLACE_PHRASE}
                  </code>{" "}
                  below to confirm.
                </p>
                <Input
                  type="text"
                  placeholder="Type the phrase above"
                  value={replaceInput}
                  onChange={(e) => setReplaceInput(e.target.value)}
                  onPaste={(e) => e.preventDefault()}
                  onDrop={(e) => e.preventDefault()}
                  className="text-xs"
                />
              </div>
            )}

            <div className="flex gap-2 justify-end mt-1">
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setPendingImport(null)}
                disabled={backupBusy !== null}
              >
                Cancel
              </Button>
              <Button
                variant={importMode === "replace" ? "destructive" : "default"}
                size="sm"
                disabled={
                  backupBusy !== null ||
                  (importMode === "replace" && replaceInput !== REQUIRED_REPLACE_PHRASE)
                }
                onClick={handleImportBackup}
              >
                {backupBusy === "import"
                  ? "Importing..."
                  : importMode === "replace"
                    ? "Replace library with backup"
                    : "Merge into library"}
              </Button>
            </div>
          </div>
        )}
      </div>

      <TaxonomyManager />

      <div className="w-full max-w-md flex flex-col gap-4 rounded-2xl border border-destructive/30 bg-destructive/5 p-6 shadow-sm mb-12">
        <h2 className="text-sm font-bold text-destructive flex items-center gap-2">
          <AlertTriangle className="size-4" />
          Danger Zone
        </h2>
        <div className="flex flex-col gap-2">
          <p className="text-sm text-muted-foreground">
            Irreversibly delete all imported questions, mark schemes, and generated content from your local database.
          </p>
          {!confirmingClear ? (
            <Button 
              variant="destructive" 
              className="mt-2 w-fit gap-2"
              onClick={() => setConfirmingClear(true)}
            >
              <Trash2 className="size-4" />
              Clear Repository
            </Button>
          ) : (
            <div className="flex flex-col gap-3 mt-2 bg-background p-4 rounded-xl border border-destructive/20">
              <p className="text-xs font-medium text-foreground">
                Type <code className="bg-muted px-1.5 py-0.5 rounded text-destructive select-all">{REQUIRED_CLEAR_PHRASE}</code> below to confirm.
              </p>
              <Input
                type="text"
                placeholder="Type the phrase above"
                value={clearInput}
                onChange={(e) => setClearInput(e.target.value)}
                onPaste={(e) => e.preventDefault()}
                onDrop={(e) => e.preventDefault()}
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
                >
                  Cancel
                </Button>
                <Button 
                  variant="destructive" 
                  size="sm"
                  disabled={clearInput !== REQUIRED_CLEAR_PHRASE || clearing}
                  onClick={handleClearRepository}
                >
                  {clearing ? "Deleting..." : "Permanently Delete All"}
                </Button>
              </div>
            </div>
          )}
        </div>
      </div>
    </section>
  );
}
