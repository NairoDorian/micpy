import React from 'react';
import { AlertCircle } from 'lucide-react';

/** React child wrapper type for the ErrorBoundary. */
interface Props {
  children: React.ReactNode;
}

/** Captured error state for the ErrorBoundary. */
interface State {
  hasError: boolean;
  error: Error | null;
}

/**
 * Class-based error boundary — must be a class component because React error
 * boundaries require `getDerivedStateFromError` / `componentDidCatch` lifecycle
 * methods, which function components cannot provide.
 * Catches unhandled render errors and displays a fallback UI with a reload button.
 */
export class ErrorBoundary extends React.Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  /** Captures the thrown error so the next render shows fallback UI. */
  static getDerivedStateFromError(error: Error) {
    return { hasError: true, error };
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="card" style={{ margin: '24px auto', maxWidth: '600px', textAlign: 'center', padding: '40px' }}>
          <AlertCircle size={40} color="#ef4444" style={{ marginBottom: '16px' }} />
          <h2 style={{ fontSize: '18px', marginBottom: '8px' }}>Something went wrong</h2>
          <p style={{ fontSize: '13px', color: 'var(--text-muted)', marginBottom: '16px' }}>
            {this.state.error?.message || 'An unexpected error occurred.'}
          </p>
          <button
            className="btn btn-primary"
            onClick={() => { this.setState({ hasError: false, error: null }); window.location.reload(); }}
          >
            Reload Application
          </button>
        </div>
      );
    }

    return this.props.children;
  }
}
