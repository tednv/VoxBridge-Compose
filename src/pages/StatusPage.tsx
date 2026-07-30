import { IconAlertCircle, IconBrandGithub, IconBug } from '@tabler/icons-preact';
import { open } from '@tauri-apps/plugin-shell';
import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'preact/hooks';
import { Button } from '../components/Button.tsx';
import { tabPanelPaddedStyle, tabPanelStyle } from '../theme/ui-primitives.ts';
import { tokens } from '../design-tokens.ts';

function HelpBubble({ label, text }: { label: string; text: string }) {
  const [visible, setVisible] = useState(false);
  return (
    <span style={{ position: 'relative', display: 'inline-flex' }}>
      <button type="button" aria-label={`${label}: ${text}`} onMouseEnter={() => setVisible(true)} onMouseLeave={() => setVisible(false)} onFocus={() => setVisible(true)} onBlur={() => setVisible(false)} style={{ width: '15px', height: '15px', padding: 0, borderRadius: '50%', border: '1px solid rgba(255,255,255,.18)', background: 'transparent', color: tokens.colors.textMuted, fontSize: '9px', lineHeight: '13px', cursor: 'help' }}>?</button>
      {visible && <span role="tooltip" style={{ position: 'absolute', zIndex: 50, top: '20px', left: 0, width: '260px', padding: '8px 10px', borderRadius: '7px', border: '1px solid rgba(255,138,0,.22)', background: 'rgba(18,17,15,.98)', boxShadow: '0 10px 30px rgba(0,0,0,.4)', color: '#d7d1c7', fontSize: '11px', fontWeight: 450, lineHeight: 1.4, textTransform: 'none', letterSpacing: 0, whiteSpace: 'normal', pointerEvents: 'none' }}>{text}</span>}
    </span>
  );
}

interface StatusPageProps {
  currentStatus: string;
  appVersion: string;
  modelStatus: Record<string, boolean>;
  isModelLoading: boolean;
  isComposeModelLoading: boolean;
  composeModelLoadError: string | null;
  modelLoadError: string | null;
  config: {
    transcription_mode: 'API' | 'Local';
    output_method: 'Typewriter' | 'Clipboard' | 'Compose';
    local_model_size: string;
    local_engine: string;
    hotkey: string;
    hotkey_mode: 'sticky' | 'hold' | 'continuous';
    enable_gpu: boolean;
    compose_backend: 'embedded' | 'ollama_remote';
    compose_model_path: string;
    compose_use_gpu: boolean;
    compose_ollama_url: string;
    compose_ollama_model: string;
  };
  hasUpdateAvailable: boolean;
  onOpenUpdateModal: () => void;
  onOpenHardwareReport: () => void;
}

interface SessionStats {
  bytesRecorded: number;
  wordsTranscribed: number;
  transcriptionsCount: number;
  transcribeMsTotal: number;
  gpuCount: number;
  cpuCount: number;
  apiCount: number;
  compose: {
    agentRuns: number;
    accepted: number;
    rejected: number;
    failed: number;
    msTotal: number;
  };
}

interface MemoryStatus {
  gpuDetected: boolean;
  adapterName?: string | null;
  dedicatedVramBytes?: number | null;
  gpuCurrentUsageBytes?: number | null;
  gpuAvailableVramBytes?: number | null;
  systemMemoryTotalBytes?: number | null;
  systemMemoryAvailableBytes?: number | null;
  systemCpuUsagePercent?: number | null;
  whisperEstimateBytes: number;
  composeEstimateBytes: number;
  composeMemorySource: string;
}

const formatBytes = (bytes: number) => {
  if (bytes >= 1_073_741_824) return `${(bytes / 1_073_741_824).toFixed(2)} GB`;
  if (bytes >= 1_048_576) return `${(bytes / 1_048_576).toFixed(2)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${bytes} B`;
};

const formatDuration = (milliseconds: number) => {
  if (milliseconds >= 1000) return `${(milliseconds / 1000).toFixed(2)} s`;
  return `${milliseconds} milliseconds`;
};

export function StatusPage({
  currentStatus,
  appVersion,
  modelStatus,
  isModelLoading,
  isComposeModelLoading,
  composeModelLoadError,
  modelLoadError,
  config,
  hasUpdateAvailable,
  onOpenUpdateModal,
  onOpenHardwareReport,
}: StatusPageProps) {
  const [stats, setStats] = useState<SessionStats | null>(null);
  const [memory, setMemory] = useState<MemoryStatus | null>(null);

  useEffect(() => {
    const refresh = () => {
      invoke<SessionStats>('get_session_stats').then(setStats).catch(() => {});
      invoke<MemoryStatus>('check_combined_vram').then(setMemory).catch(() => {});
    };
    refresh();
    const interval = setInterval(refresh, 3000);
    return () => clearInterval(interval);
  }, []);

  useEffect(() => {
    // Provider/model switches change the projected local-memory allocation
    // immediately. Clear the old projection and recalculate now rather than showing
    // the previous backend's number until the next periodic sample.
    setMemory(null);
    invoke<MemoryStatus>('check_combined_vram').then(setMemory).catch(() => {});
  }, [
    config.enable_gpu,
    config.local_model_size,
    config.compose_backend,
    config.compose_model_path,
    config.compose_use_gpu,
    config.compose_ollama_url,
    config.compose_ollama_model,
  ]);

  const transcriptionReady = Boolean(modelStatus[config.local_model_size]);
  const composeReady = config.compose_backend === 'embedded'
    ? Boolean(config.compose_model_path)
    : Boolean(config.compose_ollama_model);
  const pipelineReady = transcriptionReady && composeReady && !isModelLoading && !isComposeModelLoading && !composeModelLoadError;
  const runtimeBusy = currentStatus !== 'Ready' && currentStatus !== 'Error';
  const transcriptionAverage = stats?.transcriptionsCount
    ? stats.transcribeMsTotal / stats.transcriptionsCount
    : 0;
  const composeAverage = stats?.compose.agentRuns
    ? stats.compose.msTotal / stats.compose.agentRuns
    : 0;
  const acceptanceRate = stats?.compose.agentRuns
    ? (stats.compose.accepted / stats.compose.agentRuns) * 100
    : 0;
  const requestCount = (stats?.transcriptionsCount ?? 0) + (stats?.compose.agentRuns ?? 0);
  const systemTotal = memory?.systemMemoryTotalBytes ?? 0;
  const systemAvailable = memory?.systemMemoryAvailableBytes ?? 0;
  const systemUsed = Math.max(0, systemTotal - systemAvailable);
  const systemPercent = systemTotal ? Math.min(100, (systemUsed / systemTotal) * 100) : 0;
  const cpuPercent = Math.max(0, Math.min(100, memory?.systemCpuUsagePercent ?? 0));
  const graphicsTotal = memory?.dedicatedVramBytes ?? 0;
  const graphicsActual = memory?.gpuCurrentUsageBytes ?? 0;
  const graphicsActualPercent = graphicsTotal ? Math.min(100, (graphicsActual / graphicsTotal) * 100) : 0;
  // Keep this provider-sensitive: adapter-wide usage often remains resident for a
  // while after a backend switch and therefore looks frozen. The graph represents the
  // active VoxBridge pipeline allocation; local Ollama contributes its reported model
  // allocation, while embedded and Whisper use model-derived estimates.
  const graphicsUsed = (memory?.whisperEstimateBytes ?? 0) + (memory?.composeEstimateBytes ?? 0);
  const whisperPercent = graphicsTotal ? Math.min(100, ((memory?.whisperEstimateBytes ?? 0) / graphicsTotal) * 100) : 0;
  const refinementPercent = graphicsTotal ? Math.min(100, ((memory?.composeEstimateBytes ?? 0) / graphicsTotal) * 100) : 0;

  const metrics = [
    ['Audio captured', formatBytes(stats?.bytesRecorded ?? 0), 'Audio data processed during this application session.'],
    ['Words recognized', (stats?.wordsTranscribed ?? 0).toLocaleString(), 'Words produced by speech recognition before agent refinement.'],
    ['Recognition runs', (stats?.transcriptionsCount ?? 0).toLocaleString(), 'Completed speech-recognition requests during this session.'],
    ['Pipeline requests', requestCount.toLocaleString(), 'Combined speech-recognition and agent-refinement requests.'],
    ['Agent runs', (stats?.compose.agentRuns ?? 0).toLocaleString(), 'Individual text-refinement agent executions.'],
    ['Accepted', (stats?.compose.accepted ?? 0).toLocaleString(), 'Agent results accepted after fidelity and length checks.'],
    ['Rejected', (stats?.compose.rejected ?? 0).toLocaleString(), 'Agent results rejected by fidelity or excessive-length safeguards.'],
    ['Failed', (stats?.compose.failed ?? 0).toLocaleString(), 'Agent executions that ended with an error.'],
    ['Acceptance', `${acceptanceRate.toFixed(1)}%`, 'Percentage of agent results accepted by the refinement safeguards.'],
    ['Recognition average', formatDuration(transcriptionAverage), 'Average speech-recognition processing time per recording.'],
    ['Refinement average', formatDuration(composeAverage), 'Average processing time for each text-refinement agent run.'],
    ['Graphics / processor runs', `${stats?.gpuCount ?? 0} / ${stats?.cpuCount ?? 0}`, 'Speech-recognition runs handled with graphics acceleration versus the main processor.'],
  ];

  const runtimeRows = [
    ['Application', currentStatus, currentStatus === 'Error' ? tokens.colors.error : runtimeBusy ? tokens.colors.accentHover : tokens.colors.success, 'Overall recording and processing state.'],
    ['Speech recognition', `${config.local_engine} · ${config.local_model_size}`, transcriptionReady && !isModelLoading ? tokens.colors.success : tokens.colors.accentHover, 'The local model that converts recorded speech into text.'],
    ['Text refinement', config.compose_backend === 'embedded'
      ? 'Embedded'
      : `Ollama · ${config.compose_ollama_model || 'No model'}`, composeModelLoadError ? tokens.colors.error : composeReady && !isComposeModelLoading ? tokens.colors.success : tokens.colors.accentHover, 'The model provider used by agents to clean and rewrite recognized text.'],
    ['Pipeline', pipelineReady ? 'Operational' : composeModelLoadError ? composeModelLoadError : runtimeBusy ? currentStatus : 'Unavailable', pipelineReady ? tokens.colors.success : composeModelLoadError ? tokens.colors.error : tokens.colors.accentHover, 'Combined readiness of speech recognition, text refinement, and the active agent chain.'],
  ] as const;

  return (
    <div style={{ ...tabPanelStyle, overflow: 'auto' }} key="status">
      <div style={{ ...tabPanelPaddedStyle, gap: '14px', paddingBottom: '40px' }}>
        {modelLoadError && (
          <div style={{ display: 'flex', alignItems: 'center', gap: '10px', padding: '10px 12px', border: '1px solid rgba(239,102,94,.35)', borderRadius: tokens.radii.input, color: tokens.colors.error, background: 'rgba(239,102,94,.08)', fontSize: tokens.typography.sizeSm }}>
            <IconAlertCircle size={17} />
            <span style={{ flex: 1 }}>{modelLoadError}</span>
            <Button variant="ghost" size="sm" onClick={onOpenHardwareReport}>Hardware report</Button>
          </div>
        )}

        <section style={{ display: 'grid', gridTemplateColumns: 'repeat(4, minmax(0, 1fr))', border: '1px solid rgba(255,255,255,.08)', borderRadius: tokens.radii.panel, overflow: 'hidden', background: 'rgba(15,15,13,.7)' }}>
          {runtimeRows.map(([label, value, stateColor, help], index) => (
            <div key={label} style={{ padding: '13px 15px', borderLeft: index ? '1px solid rgba(255,255,255,.08)' : 'none' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: '7px', color: tokens.colors.textMuted, fontSize: '10px', textTransform: 'uppercase', letterSpacing: '.08em' }}>
                <span style={{ width: '6px', height: '6px', borderRadius: '50%', background: typeof stateColor === 'string' ? stateColor : stateColor ? tokens.colors.success : tokens.colors.accentHover }} />
                {label}
                <HelpBubble label={label} text={help} />
              </div>
              <div style={{ marginTop: '6px', color: tokens.colors.textPrimary, fontSize: tokens.typography.sizeXs, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{value}</div>
            </div>
          ))}
        </section>

        <section style={{ display: 'grid', gridTemplateColumns: 'repeat(2, minmax(0, 1fr))', gap: '12px' }}>
          <div style={{ padding: '15px 16px', border: '1px solid rgba(255,255,255,.08)', borderRadius: tokens.radii.panel, background: 'rgba(255,255,255,.012)' }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', gap: '12px', alignItems: 'center' }}>
              <span style={{ display: 'inline-flex', alignItems: 'center', gap: '7px', color: tokens.colors.textPrimary, fontSize: tokens.typography.sizeSm, fontWeight: 700 }}>
                Processor activity
                <HelpBubble label="Processor activity" text="Live activity across all logical processors. This is total system use, not only VoxBridge Compose." />
              </span>
              <span style={{ color: tokens.colors.textMuted, fontFamily: tokens.typography.fontMono, fontSize: tokens.typography.sizeXs }}>
                {memory?.systemCpuUsagePercent != null ? `${cpuPercent.toFixed(1)}%` : 'Sampling'}
              </span>
            </div>
            <div style={{ height: '9px', marginTop: '12px', overflow: 'hidden', borderRadius: '999px', background: 'rgba(255,255,255,.07)' }}>
              <div style={{ width: `${cpuPercent}%`, height: '100%', borderRadius: 'inherit', background: 'linear-gradient(90deg, #ff8a00, #ffd966)', transition: 'width .25s ease' }} />
            </div>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, minmax(0, 1fr))', gap: '8px', marginTop: '10px' }}>
              <div style={{ padding: '8px 9px', borderRadius: '7px', background: 'rgba(255,138,0,.06)' }}>
                <div style={{ color: tokens.colors.textMuted, fontSize: '10px' }}>Graphics / processor runs</div>
                <div style={{ marginTop: '3px', color: tokens.colors.textPrimary, fontFamily: tokens.typography.fontMono, fontSize: '12px' }}>{stats?.gpuCount ?? 0} / {stats?.cpuCount ?? 0}</div>
              </div>
              <div style={{ padding: '8px 9px', borderRadius: '7px', background: 'rgba(255,184,0,.06)' }}>
                <div style={{ color: tokens.colors.textMuted, fontSize: '10px' }}>Recognition average</div>
                <div style={{ marginTop: '3px', color: tokens.colors.textPrimary, fontFamily: tokens.typography.fontMono, fontSize: '12px' }}>{formatDuration(transcriptionAverage)}</div>
              </div>
            </div>
            <div style={{ marginTop: '8px', color: tokens.colors.textMuted, fontSize: '11px' }}>
              {requestCount.toLocaleString()} pipeline requests this session
            </div>
          </div>

          <div style={{ padding: '15px 16px', border: '1px solid rgba(255,255,255,.08)', borderRadius: tokens.radii.panel, background: 'rgba(255,255,255,.012)' }}>
            <div style={{ display: 'flex', justifyContent: 'space-between', gap: '12px', alignItems: 'center' }}>
              <span style={{ display: 'inline-flex', alignItems: 'center', gap: '7px', color: tokens.colors.textPrimary, fontSize: tokens.typography.sizeSm, fontWeight: 700 }}>
                Graphics memory
                <HelpBubble label="Graphics memory estimate" text="This graph estimates memory assigned to the active VoxBridge pipeline, so it recalculates when you switch refinement providers. Whisper and embedded refinement use model-size estimates. A local Ollama server may report its loaded model allocation; an Ollama server on another computer is not included." />
              </span>
              <span style={{ color: tokens.colors.textMuted, fontFamily: tokens.typography.fontMono, fontSize: tokens.typography.sizeXs }}>
                {graphicsTotal ? `${formatBytes(graphicsUsed)} estimated / ${formatBytes(graphicsTotal)}` : 'Unavailable'}
              </span>
            </div>
            <div style={{ display: 'flex', height: '9px', marginTop: '12px', overflow: 'hidden', borderRadius: '999px', background: 'rgba(255,255,255,.07)' }}>
              <div title="Whisper estimate" style={{ width: `${whisperPercent}%`, height: '100%', background: '#ff8a00', transition: 'width .25s ease' }} />
              <div title="Refinement estimate" style={{ width: `${refinementPercent}%`, height: '100%', background: '#ffd966', transition: 'width .25s ease' }} />
            </div>
            <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2, minmax(0, 1fr))', gap: '8px', marginTop: '10px' }}>
              <div style={{ padding: '8px 9px', borderRadius: '7px', background: 'rgba(255,138,0,.06)' }}>
                <div style={{ color: tokens.colors.textMuted, fontSize: '10px' }}>Whisper projected allocation</div>
                <div style={{ marginTop: '3px', color: tokens.colors.textPrimary, fontFamily: tokens.typography.fontMono, fontSize: '12px' }}>{formatBytes(memory?.whisperEstimateBytes ?? 0)} · {whisperPercent.toFixed(1)}%</div>
              </div>
              <div style={{ padding: '8px 9px', borderRadius: '7px', background: 'rgba(255,184,0,.06)' }}>
                <div style={{ color: tokens.colors.textMuted, fontSize: '10px' }}>{memory?.composeMemorySource || 'Recalculating refinement allocation'}</div>
                <div style={{ marginTop: '3px', color: tokens.colors.textPrimary, fontFamily: tokens.typography.fontMono, fontSize: '12px' }}>{formatBytes(memory?.composeEstimateBytes ?? 0)} · {refinementPercent.toFixed(1)}%</div>
              </div>
            </div>
            <div style={{ marginTop: '8px', color: tokens.colors.textMuted, fontSize: '11px', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
              {memory?.adapterName || 'Local graphics adapter not detected'}
            </div>
            {graphicsTotal > 0 && (
              <div style={{ display: 'flex', justifyContent: 'space-between', gap: '12px', marginTop: '5px', color: tokens.colors.textMuted, fontSize: '10px' }}>
                <span>Adapter allocation {formatBytes(graphicsActual)} · {graphicsActualPercent.toFixed(1)}%</span>
                <span>{formatBytes(memory?.gpuAvailableVramBytes ?? 0)} available</span>
              </div>
            )}
          </div>
        </section>

        <section style={{ padding: '15px 16px', border: '1px solid rgba(255,255,255,.08)', borderRadius: tokens.radii.panel, background: 'rgba(255,255,255,.012)' }}>
          <div style={{ display: 'flex', justifyContent: 'space-between', gap: '12px', alignItems: 'center' }}>
            <span style={{ display: 'inline-flex', alignItems: 'center', gap: '7px', color: tokens.colors.textPrimary, fontSize: tokens.typography.sizeSm, fontWeight: 700 }}>
              System memory
              <HelpBubble label="System memory" text="Live physical memory use for the entire computer. Local speech and embedded refinement models contribute to this total when they run on the processor." />
            </span>
            <span style={{ color: tokens.colors.textMuted, fontFamily: tokens.typography.fontMono, fontSize: tokens.typography.sizeXs }}>
              {systemTotal ? `${formatBytes(systemUsed)} / ${formatBytes(systemTotal)}` : 'Unavailable'}
            </span>
          </div>
          <div style={{ height: '9px', marginTop: '12px', overflow: 'hidden', borderRadius: '999px', background: 'rgba(255,255,255,.07)' }}>
            <div style={{ width: `${systemPercent}%`, height: '100%', borderRadius: 'inherit', background: 'linear-gradient(90deg, #ff8a00, #ffd966)', transition: 'width .25s ease' }} />
          </div>
          <div style={{ display: 'flex', justifyContent: 'space-between', gap: '12px', marginTop: '8px', color: tokens.colors.textMuted, fontSize: '11px' }}>
            <span>{systemTotal ? `${systemPercent.toFixed(1)}% used` : 'System memory reporting is unavailable.'}</span>
            {systemTotal > 0 && <span>{formatBytes(systemAvailable)} available</span>}
          </div>
        </section>

        <section style={{ display: 'grid', gridTemplateColumns: 'repeat(4, minmax(0, 1fr))', borderTop: '1px solid rgba(255,255,255,.08)', borderLeft: '1px solid rgba(255,255,255,.08)' }}>
          {metrics.map(([label, value, help]) => (
            <div key={label} style={{ minHeight: '86px', padding: '14px 16px', borderRight: '1px solid rgba(255,255,255,.08)', borderBottom: '1px solid rgba(255,255,255,.08)', background: 'rgba(255,255,255,.012)' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: '6px', color: tokens.colors.textMuted, fontSize: '10px', textTransform: 'uppercase', letterSpacing: '.08em' }}>
                {label}
                <HelpBubble label={label} text={help} />
              </div>
              <div style={{ marginTop: '9px', color: tokens.colors.textPrimary, fontFamily: tokens.typography.fontMono, fontSize: '20px', fontWeight: 700 }}>{value}</div>
            </div>
          ))}
        </section>

        <footer style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', color: tokens.colors.textMuted, fontSize: tokens.typography.sizeXs }}>
          <span>VoxBridge Compose v{appVersion}</span>
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
            <Button variant="ghost" size="sm" onClick={onOpenHardwareReport} title="Review a sanitized hardware report and open a GitHub issue">
              <IconBug size={15} /> Report a bug
            </Button>
            {hasUpdateAvailable && <Button variant="ghost" size="sm" onClick={onOpenUpdateModal}>Update available</Button>}
            <Button variant="ghost" size="sm" onClick={() => open('https://github.com/tednv/VoxBridge-Compose')} title="Open repository">
              <IconBrandGithub size={15} /> Repository
            </Button>
          </div>
        </footer>
      </div>
    </div>
  );
}
