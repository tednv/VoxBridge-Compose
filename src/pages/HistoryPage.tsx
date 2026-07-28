import { tabPanelPaddedStyle, tabPanelStyle } from '../theme/ui-primitives.ts';
import { tokens } from '../design-tokens.ts';

interface HistoryItem {
  id: number;
  text: string;
  timestamp: string;
}

interface HistoryPageProps {
  history: HistoryItem[];
  onClear: () => void;
}

export function HistoryPage({ history, onClear }: HistoryPageProps) {
  return (
    <div style={{ ...tabPanelStyle, height: '100%', minHeight: 0, overflow: 'hidden', position: 'relative' }} key="history">
      <div style={{ ...tabPanelPaddedStyle, height: '100%', minHeight: 0, paddingBottom: '42px' }}>
        {history.length === 0 ? (
          <p style={{ color: tokens.colors.textMuted, padding: '32px 2px' }}>No history yet.</p>
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0, overflowY: 'auto' }}>
            {history.map((item, index) => (
              <article key={item.id} style={{ display: 'grid', gridTemplateColumns: '180px minmax(0, 1fr)', gap: '24px', padding: '20px 2px', borderBottom: '1px solid rgba(255,255,255,.07)' }}>
                <div>
                  <div style={{ color: tokens.colors.accentPrimary, fontFamily: tokens.typography.fontMono, fontSize: '10px' }}>#{String(history.length - index).padStart(3, '0')}</div>
                  <time style={{ display: 'block', marginTop: '5px', color: tokens.colors.textMuted, fontSize: tokens.typography.sizeXs }}>{new Date(item.timestamp).toLocaleString()}</time>
                </div>
                <p style={{ margin: 0, color: tokens.colors.textPrimary, fontSize: tokens.typography.sizeMd, lineHeight: 1.7, whiteSpace: 'pre-wrap' }}>{item.text}</p>
              </article>
            ))}
          </div>
        )}
        <button
          type="button"
          onClick={onClear}
          disabled={history.length === 0}
          style={{ position: 'absolute', right: '24px', bottom: '14px', padding: '4px 6px', border: 0, background: 'rgba(10,10,9,.92)', color: history.length > 0 ? tokens.colors.textMuted : 'rgba(231,226,216,.25)', fontSize: '11px', cursor: history.length > 0 ? 'pointer' : 'default' }}
        >
          Clear history
        </button>
      </div>
    </div>
  );
}
