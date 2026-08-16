import { Component, ErrorInfo, ReactNode } from "react";
import { AlertCircle, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";

interface Props {
  children: ReactNode;
  fallbackTitle?: string;
}

interface State {
  hasError: boolean;
  error: Error | null;
  errorInfo: ErrorInfo | null;
}

export class ErrorBoundary extends Component<Props, State> {
  public state: State = {
    hasError: false,
    error: null,
    errorInfo: null,
  };

  public static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error, errorInfo: null };
  }

  public componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    console.error("[MergeMark][ErrorBoundary] Uncaught component error:", error, errorInfo);
    this.setState({ errorInfo });
  }

  private handleReset = () => {
    this.setState({ hasError: false, error: null, errorInfo: null });
    window.location.reload();
  };

  public render() {
    if (this.state.hasError) {
      return (
        <div className="flex flex-col items-center justify-center min-h-[400px] h-full w-full p-8 text-center bg-background text-foreground select-none">
          <div className="size-14 rounded-2xl bg-destructive/10 border border-destructive/20 flex items-center justify-center text-destructive mb-4 shadow-lg">
            <AlertCircle className="size-7" />
          </div>

          <h2 className="text-xl font-bold tracking-tight text-foreground mb-1">
            {this.props.fallbackTitle || "Something went wrong"}
          </h2>
          <p className="text-xs text-muted-foreground max-w-md mb-6">
            An unexpected error occurred while rendering this view. You can reload the interface or check the diagnostic details below.
          </p>

          <div className="flex items-center gap-3 mb-6">
            <Button
              variant="default"
              size="sm"
              onClick={this.handleReset}
              className="gap-2 text-xs font-semibold"
            >
              <RefreshCw className="size-3.5" />
              <span>Reload Application</span>
            </Button>
          </div>

          {this.state.error && (
            <div className="w-full max-w-xl text-left bg-muted/40 border border-border/80 rounded-xl p-4 overflow-hidden">
              <span className="text-[11px] font-mono font-semibold text-destructive block mb-1">
                {this.state.error.toString()}
              </span>
              {this.state.errorInfo?.componentStack && (
                <pre className="text-[10px] font-mono text-muted-foreground overflow-x-auto max-h-40 whitespace-pre-wrap">
                  {this.state.errorInfo.componentStack}
                </pre>
              )}
            </div>
          )}
        </div>
      );
    }

    return this.props.children;
  }
}
