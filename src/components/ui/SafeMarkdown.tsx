import { Component, type ErrorInfo, type ReactNode } from "react";

interface SafeMarkdownProps {
  children: ReactNode;
  rawText: string;
  fallback?: ReactNode;
}

interface SafeMarkdownState {
  failed: boolean;
}

/** Prevent one malformed KaTeX node from taking down an entire card. */
export class SafeMarkdown extends Component<SafeMarkdownProps, SafeMarkdownState> {
  state: SafeMarkdownState = { failed: false };

  static getDerivedStateFromError(): SafeMarkdownState {
    return { failed: true };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("Markdown/KaTeX render failed; showing raw content", error, info);
  }

  componentDidUpdate(previous: SafeMarkdownProps) {
    if (this.state.failed && previous.rawText !== this.props.rawText) {
      this.setState({ failed: false });
    }
  }

  render() {
    if (this.state.failed) {
      if (this.props.fallback) {
        return (
          <div key={`fallback-${this.props.rawText}`} className="contents">
            {this.props.fallback}
          </div>
        );
      }
      return (
        <pre
          key={`error-${this.props.rawText}`}
          className="whitespace-pre-wrap break-words font-sans text-inherit"
        >
          {this.props.rawText}
        </pre>
      );
    }

    return (
      <div key={`content-${this.props.rawText}`} className="contents">
        {this.props.children}
      </div>
    );
  }
}
