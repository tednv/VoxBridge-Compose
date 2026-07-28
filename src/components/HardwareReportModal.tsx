import { useEffect, useState } from 'preact/hooks';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-shell';
import { IconBrandGithub, IconCopy, IconCheck } from '@tabler/icons-preact';
import { Modal } from './Modal.tsx';
import { Button } from './Button.tsx';
import { tokens } from '../design-tokens.ts';

interface HardwareReportModalProps {
  onClose: () => void;
}

const GITHUB_ISSUES_URL = 'https://github.com/tednv/VoxBridge-Compose/issues/new';

function sanitizeReport(text: string): string {
  return text
    .replace(/[A-Z]:\\Users\\[^\\\r\n]+/gi, 'USER_HOME')
    .replace(/\/home\/[^/\r\n]+/g, 'USER_HOME')
    .replace(/\/Users\/[^/\r\n]+/g, 'USER_HOME');
}

export function HardwareReportModal({ onClose }: HardwareReportModalProps) {
  const [report, setReport] = useState<string>('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    invoke<string>('get_hardware_report')
      .then((text) => {
        setReport(sanitizeReport(text));
        setLoading(false);
      })
      .catch((err) => {
        setError(String(err));
        setLoading(false);
      });
  }, []);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(report);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      // Clipboard permission denied or unavailable — the textarea is still selectable
      // as a manual fallback, so this isn't fatal.
    }
  };

  const handleOpenIssue = () => {
    // Try to prefill the issue body via GitHub's `body` query param. Very long reports
    // can get truncated/stripped by some browsers, so the Copy button above is always
    // available as a guaranteed-to-work fallback ("paste it yourself").
    const body = `## Hardware report\n\n\`\`\`text\n${report}\n\`\`\`\n\n## What happened\n\nDescribe the problem and the steps that triggered it.`;
    const url = `${GITHUB_ISSUES_URL}?title=${encodeURIComponent('Hardware compatibility report')}&body=${encodeURIComponent(body)}`;
    open(url);
  };

  return (
    <Modal title="Report hardware" onClose={onClose} maxWidth="560px">
      <div style={{ fontSize: tokens.typography.sizeSm, color: tokens.colors.textSecondary, lineHeight: 1.5 }}>
        Review the sanitized processor, graphics, operating-system, and recent graphics-failure details below.
        Nothing is submitted until you choose to open GitHub and publish the issue.
      </div>

      {loading ? (
        <div style={{ padding: tokens.spacing.md, color: tokens.colors.textMuted, fontSize: tokens.typography.sizeSm }}>
          Gathering hardware info...
        </div>
      ) : error ? (
        <div style={{ padding: tokens.spacing.md, color: tokens.colors.error, fontSize: tokens.typography.sizeSm }}>
          Failed to gather hardware info: {error}
        </div>
      ) : (
        <textarea
          readOnly
          value={report}
          onClick={(e: MouseEvent) => (e.target as HTMLTextAreaElement).select()}
          style={{
            width: '100%',
            minHeight: '220px',
            resize: 'vertical',
            background: tokens.colors.bgPrimary,
            color: tokens.colors.textPrimary,
            border: '1px solid rgba(255, 255, 255, 0.12)',
            borderRadius: tokens.radii.input,
            padding: tokens.spacing.sm,
            fontFamily: tokens.typography.fontMono,
            fontSize: tokens.typography.sizeXs,
            lineHeight: 1.5,
            boxSizing: 'border-box',
          }}
        />
      )}

      <div style={{ display: 'flex', justifyContent: 'flex-end', gap: tokens.spacing.sm, marginTop: tokens.spacing.sm }}>
        <Button variant="ghost" onClick={handleCopy} disabled={loading || !!error}>
          {copied ? <IconCheck size={16} /> : <IconCopy size={16} />}
          {copied ? 'Copied' : 'Copy'}
        </Button>
        <Button variant="secondary" onClick={handleOpenIssue} disabled={loading || !!error}>
          <IconBrandGithub size={16} />
          Open bug report
        </Button>
      </div>
    </Modal>
  );
}
