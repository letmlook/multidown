import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false, error: null };

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("ErrorBoundary:", error, info.componentStack);
  }

  render() {
    if (this.state.hasError && this.state.error) {
      return (
        <div style={{
          padding: 24,
          fontFamily: "system-ui, sans-serif",
          color: "#333",
          background: "#fff",
          minHeight: "100vh",
        }}>
          <h2 style={{ marginBottom: 12, color: "#c00" }}>界面加载出错</h2>
          <pre style={{
            padding: 12,
            background: "#f5f5f5",
            borderRadius: 6,
            overflow: "auto",
            fontSize: 13,
            whiteSpace: "pre-wrap",
            wordBreak: "break-all",
          }}>
            {this.state.error.message}
          </pre>
          <p style={{ marginTop: 12, fontSize: 13, color: "#666" }}>
            请尝试重新启动应用或检查控制台获取更多信息。
          </p>
        </div>
      );
    }
    return this.props.children;
  }
}
