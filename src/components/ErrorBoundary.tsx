import { Component, type ReactNode } from "react";

interface State {
  error: Error | null;
}

export class ErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { error: null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  componentDidCatch(error: Error, info: { componentStack: string | null }) {
    console.error("UI crash:", error, info?.componentStack);
  }
  render() {
    if (this.state.error) {
      return (
        <div className="min-h-screen flex items-center justify-center p-6 bg-background text-foreground">
          <div className="max-w-xl w-full rounded-lg border border-destructive/40 bg-card p-6 space-y-3">
            <div className="text-destructive font-semibold">Что-то отвалилось в UI</div>
            <div className="text-xs text-muted-foreground font-mono whitespace-pre-wrap break-words">
              {this.state.error.message}
            </div>
            <details className="text-[11px] text-muted-foreground/70">
              <summary className="cursor-pointer">stack</summary>
              <pre className="whitespace-pre-wrap break-words mt-2 max-h-[300px] overflow-y-auto">
                {this.state.error.stack}
              </pre>
            </details>
            <button onClick={() => location.reload()} className="text-xs underline text-foreground">
              Перезагрузить
            </button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}
