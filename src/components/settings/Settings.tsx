import { useState, useEffect } from "react";
import { Settings as SettingsIcon, Key, RefreshCw, AlertCircle } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { invoke } from "@tauri-apps/api/core";


export function Settings() {
  const [apiKey, setApiKey] = useState("");
  const [baseUrl, setBaseUrl] = useState("https://api.openai.com/v1");
  const [modelName, setModelName] = useState("gpt-4o-mini");
  const [availableModels, setAvailableModels] = useState<string[]>([]);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [modelFetchError, setModelFetchError] = useState("");

  useEffect(() => {
    const savedKey = localStorage.getItem("mergemark_openai_key");
    if (savedKey) setApiKey(savedKey);

    const savedBaseUrl = localStorage.getItem("mergemark_openai_base_url");
    if (savedBaseUrl) setBaseUrl(savedBaseUrl);

    const savedModel = localStorage.getItem("mergemark_openai_model");
    if (savedModel) setModelName(savedModel);
  }, []);

  function handleKeyChange(e: React.ChangeEvent<HTMLInputElement>) {
    const newKey = e.target.value;
    setApiKey(newKey);
    localStorage.setItem("mergemark_openai_key", newKey);
  }

  function handleBaseUrlChange(e: React.ChangeEvent<HTMLInputElement>) {
    const newUrl = e.target.value;
    setBaseUrl(newUrl);
    localStorage.setItem("mergemark_openai_base_url", newUrl);
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

  return (
    <section
      className="flex flex-1 flex-col items-center justify-center h-full min-h-0 px-8 py-12 bg-background overflow-y-auto"
      aria-label="Settings"
    >
      <div className="mb-8 text-center space-y-1 select-none">
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
    </section>
  );
}
