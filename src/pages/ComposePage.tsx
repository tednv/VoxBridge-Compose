import { useEffect, useRef, useState } from 'preact/hooks';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { IconCopy, IconTrash, IconRefresh, IconHistory, IconChevronDown, IconChevronUp, IconDownload, IconFolderOpen, IconMicrophone, IconPlayerStopFilled } from '@tabler/icons-preact';
import { Card } from '../components/Card.tsx';
import { Button } from '../components/Button.tsx';
import { tabPanelPaddedStyle, tabPanelStyle } from '../theme/ui-primitives.ts';
import { tokens } from '../design-tokens.ts';

interface ComposeState {
  rawText: string;
  text: string;
  correcting: boolean;
  activeAgent: string | null;
}

interface AgentStage {
  agentId: string;
  agentName: string;
  text: string;
  accepted: boolean;
  note: string | null;
  fidelity: number;
}

interface BatchRecord {
  id: number;
  rawText: string;
  stages: AgentStage[];
}

interface ComposeAgentSummary {
  id: string;
  name: string;
  prompt: string;
  enabled: boolean;
  priority: number;
  speed: 'low' | 'medium' | 'high';
  presetId?: string | null;
  minFidelity: number;
  includeHistory?: boolean;
  historyItems?: number;
}

interface AgentPresetSummary {
  id: string;
  name: string;
}

interface ComposePageProps {
  onCopyToClipboard: (text: string) => void;
  onEditAgent: (agentId: string) => void;
  currentStatus: string;
  enginesReady: boolean;
}

interface ComposeDumpResult {
  fileName: string;
  filePath: string;
}

function AgentHelp({ label, text }: { label: string; text: string }) {
  const [visible, setVisible] = useState(false);
  return (
    <span style={{ position: 'relative', display: 'inline-flex', alignItems: 'center' }}>
      <button
        type="button"
        aria-label={`${label}: ${text}`}
        onMouseEnter={() => setVisible(true)}
        onMouseLeave={() => setVisible(false)}
        onFocus={() => setVisible(true)}
        onBlur={() => setVisible(false)}
        onClick={(event) => event.stopPropagation()}
        style={{ width: '15px', height: '15px', padding: 0, borderRadius: '50%', border: '1px solid rgba(255,255,255,.2)', background: 'transparent', color: tokens.colors.textMuted, fontSize: '9px', lineHeight: '13px', cursor: 'help' }}
      >
        ?
      </button>
      {visible && (
        <span role="tooltip" style={{ position: 'absolute', zIndex: 60, top: '20px', left: 0, width: '260px', padding: '8px 10px', borderRadius: '7px', border: '1px solid rgba(255,138,0,.24)', background: 'rgba(18,17,15,.99)', boxShadow: '0 10px 30px rgba(0,0,0,.45)', color: '#d7d1c7', fontSize: '11px', fontWeight: 450, lineHeight: 1.4, whiteSpace: 'normal', pointerEvents: 'none' }}>
          {text}
        </span>
      )}
    </span>
  );
}

interface OffloadConfig {
  offload_locations: string[];
  default_offload_location: string;
  remember_offload_location: boolean;
  last_offload_location: string;
  hotkey: string;
}

// Mirrors compose::filename_slug in Rust exactly, so the live preview shown while typing
// matches the actual filename the backend will write on offload - first 8 words, alphanumeric
// only, lowercased, hyphen-joined, capped at 50 chars on a word boundary.
function generateFilenameSlug(text: string): string {
  const words = text
    .split(/\s+/)
    .slice(0, 8)
    .map((w) => w.replace(/[^a-zA-Z0-9]/g, '').toLowerCase())
    .filter((w) => w.length > 0);

  if (words.length === 0) return 'dictation';

  let slug = words.join('-');
  if (slug.length > 50) {
    const cut = slug.lastIndexOf('-', 50);
    slug = slug.slice(0, cut > 0 ? cut : 50);
  }
  return slug;
}

export function ComposePage({ onCopyToClipboard, onEditAgent, currentStatus, enginesReady }: ComposePageProps) {
  const [rawText, setRawText] = useState('');
  const [text, setText] = useState('');
  const [correcting, setCorrecting] = useState(false);
  const [activeAgent, setActiveAgent] = useState<string | null>(null);
  const [history, setHistory] = useState<BatchRecord[]>([]);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [expandedBatchId, setExpandedBatchId] = useState<number | null>(null);
  const [allAgents, setAllAgents] = useState<ComposeAgentSummary[]>([]);
  const [agentPresets, setAgentPresets] = useState<AgentPresetSummary[]>([]);
  const [previewThreshold, setPreviewThreshold] = useState<number | null>(null);
  const [selectedAgentIndex, setSelectedAgentIndex] = useState(0);
  const [inspectedAgentId, setInspectedAgentId] = useState<string | null>(null);
  const [dumpStatus, setDumpStatus] = useState<string | null>(null);
  const [semanticFilename, setSemanticFilename] = useState<string | null>(null);
  const [dictationsDir, setDictationsDir] = useState<string | null>(null);
  const [offloadLocations, setOffloadLocations] = useState<string[]>([]);
  const [rememberOffloadLocation, setRememberOffloadLocation] = useState(false);
  const [recordingShortcut, setRecordingShortcut] = useState('');
  const [isApplyingThreshold, setIsApplyingThreshold] = useState(false);
  const [recordingCommandPending, setRecordingCommandPending] = useState(false);
  const originalPaneRef = useRef<HTMLDivElement | null>(null);
  const polishedPaneRef = useRef<HTMLDivElement | null>(null);
  const originalFollowsOutput = useRef(true);
  const polishedFollowsOutput = useRef(true);
  const previousStatusRef = useRef(currentStatus);
  const filenameRefreshAfterStopRef = useRef(false);
  const filenameRefreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const filenameRequestIdRef = useRef(0);

  // The live fidelity-tuning slider tunes one agent at a time - step through the chain
  // with the nav arrows below. Every OTHER agent's stage keeps its real accept/reject
  // decision from when the chain actually ran; only the selected agent's is re-evaluated
  // at the hypothetical threshold. That's an approximation for chains longer than one
  // agent (a changed decision upstream would, in reality, change what a downstream agent
  // even saw), but it's the right scope for "how would tuning THIS agent's bar affect
  // its own edits" without re-calling any LLM.
  const enabledAgents = [...allAgents].filter((a) => a.enabled).sort((a, b) => a.priority - b.priority);
  const clampedAgentIndex = Math.min(selectedAgentIndex, Math.max(0, enabledAgents.length - 1));
  const selectedAgent = enabledAgents[clampedAgentIndex] || null;
  const effectiveThreshold = previewThreshold ?? selectedAgent?.minFidelity ?? 0.85;
  const statusColor = currentStatus === 'Error'
    ? tokens.colors.error
    : currentStatus === 'Ready'
      ? tokens.colors.success
      : tokens.colors.accentHover;

  const updateFollowState = (element: HTMLDivElement, followRef: { current: boolean }) => {
    followRef.current = element.scrollHeight - element.scrollTop - element.clientHeight < 32;
  };

  useEffect(() => {
    if (originalFollowsOutput.current && originalPaneRef.current) {
      originalPaneRef.current.scrollTop = originalPaneRef.current.scrollHeight;
    }
  }, [rawText]);

  const selectAgentIndex = (index: number) => {
    setSelectedAgentIndex(index);
    setPreviewThreshold(null);
  };

  const refreshAgents = () => {
    invoke<ComposeAgentSummary[]>('get_compose_agents')
      .then(setAllAgents)
      .catch(() => {});
  };

  const refreshHistory = () => {
    invoke<BatchRecord[]>('get_compose_history')
      .then(setHistory)
      .catch(() => {});
  };

  useEffect(() => {
    invoke<ComposeState>('get_compose_text')
      .then((state) => {
        setRawText(state.rawText);
        setText(state.text);
      })
      .catch(() => {});
    refreshHistory();
    refreshAgents();
    invoke<AgentPresetSummary[]>('get_agent_presets')
      .then(setAgentPresets)
      .catch(() => {});
    Promise.all([
      invoke<string>('get_dictations_dir'),
      invoke<string>('get_default_dictations_dir'),
      invoke<OffloadConfig>('get_config'),
    ]).then(([effective, builtInDefault, loadedConfig]) => {
      setDictationsDir(effective);
      setOffloadLocations(Array.from(new Set([
        loadedConfig.default_offload_location || builtInDefault,
        ...loadedConfig.offload_locations,
        effective,
      ].filter(Boolean))));
      setRememberOffloadLocation(loadedConfig.remember_offload_location);
      setRecordingShortcut(loadedConfig.hotkey);
    }).catch(() => {});

    const unlisten = listen<ComposeState>('compose-state-updated', (event) => {
      setRawText(event.payload.rawText);
      setText(event.payload.text);
      setCorrecting(event.payload.correcting);
      setActiveAgent(event.payload.activeAgent);
      if (!event.payload.correcting) {
        refreshHistory();
      }
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  // Threshold editing remains local until the user explicitly applies it. This keeps
  // slider exploration cheap and prevents accidental downstream LLM recomputation.
  const handleThresholdChange = (value: number) => {
    setPreviewThreshold(value);
  };

  const applyThreshold = () => {
    if (!selectedAgent || previewThreshold === null) return;
    const value = previewThreshold;
    const updated = allAgents.map((agent) => (agent.id === selectedAgent.id ? { ...agent, minFidelity: value } : agent));
    setIsApplyingThreshold(true);
    invoke('save_compose_agents', { agents: updated })
      .then(() => invoke('recompute_compose_fidelity', { agentId: selectedAgent.id, threshold: value }))
      .then(() => Promise.all([
        invoke<ComposeState>('get_compose_text'),
        invoke<BatchRecord[]>('get_compose_history'),
        invoke<ComposeAgentSummary[]>('get_compose_agents'),
      ]))
      .then(([state, nextHistory, nextAgents]) => {
        setRawText(state.rawText);
        setText(state.text);
        setHistory(nextHistory);
        setAllAgents(nextAgents);
        setPreviewThreshold(null);
      })
      .catch((error) => console.error('Failed to apply fidelity threshold:', error))
      .finally(() => setIsApplyingThreshold(false));
  };

  // What the Polished pane would look like if the *selected* agent's stage were
  // re-evaluated at `effectiveThreshold`, recomputed client-side from each batch's
  // already-stored stage fidelity scores rather than by re-running anything. Every
  // OTHER agent's stage in the chain keeps its real accept/reject decision from when it
  // actually ran - only the selected agent's own decision is hypothetical, so this won't
  // reflect what a downstream agent WOULD have seen had an upstream one's real decision
  // also been different (that would need an actual re-run, not just re-thresholding).
  // The live document is authoritative. History batches may intentionally overlap
  // (for example, full-context refinement starts each batch at offset zero), so joining
  // every batch repeats the document in the visible pane. Only reconstruct a
  // hypothetical view while the user is actively previewing a changed threshold.
  const previewText = previewThreshold !== null && history.length > 0
    ? history
        .map((batch) => {
          let current = batch.rawText;
          batch.stages.forEach((stage, i) => {
            const accepted = i === clampedAgentIndex ? stage.fidelity >= effectiveThreshold : stage.accepted;
            if (accepted) current = stage.text;
          });
          return current;
        })
        .join(' ')
    : text;

  useEffect(() => {
    if (polishedFollowsOutput.current && polishedPaneRef.current) {
      polishedPaneRef.current.scrollTop = polishedPaneRef.current.scrollHeight;
    }
  }, [previewText]);

  useEffect(() => {
    const previous = previousStatusRef.current;
    previousStatusRef.current = currentStatus;
    if (previous !== 'Ready' && currentStatus === 'Ready' && text.trim()) {
      filenameRefreshAfterStopRef.current = true;
      setSemanticFilename(null);
    } else if (previous === 'Ready' && currentStatus === 'Recording') {
      setSemanticFilename(null);
    }
  }, [currentStatus, text]);

  useEffect(() => {
    if (
      !filenameRefreshAfterStopRef.current
      || currentStatus !== 'Ready'
      || correcting
      || !text.trim()
    ) {
      return;
    }

    filenameRefreshAfterStopRef.current = false;
    invoke<string>('suggest_compose_filename', { text })
      .then((slug) => setSemanticFilename(`${slug}.txt`))
      .catch(() => setSemanticFilename(`${generateFilenameSlug(text)}.txt`));
  }, [currentStatus, correcting, text]);

  // A settled rolling refinement is the natural pause signal for Compose. Wait a
  // little longer than the correction debounce, then ask the active backend for a
  // topic-based name from the complete refined document. New speech or another agent
  // pass cancels the pending request, so filename generation never runs per keystroke.
  useEffect(() => {
    if (filenameRefreshTimerRef.current) {
      clearTimeout(filenameRefreshTimerRef.current);
      filenameRefreshTimerRef.current = null;
    }
    if (correcting || currentStatus !== 'Recording' || text.trim().split(/\s+/).length < 20) {
      return;
    }

    const requestId = ++filenameRequestIdRef.current;
    filenameRefreshTimerRef.current = setTimeout(() => {
      invoke<string>('suggest_compose_filename', { text })
        .then((slug) => {
          if (filenameRequestIdRef.current === requestId) {
            setSemanticFilename(`${slug}.txt`);
          }
        })
        .catch(() => {
          if (filenameRequestIdRef.current === requestId) {
            setSemanticFilename(`${generateFilenameSlug(text)}.txt`);
          }
        });
    }, 2500);

    return () => {
      if (filenameRefreshTimerRef.current) {
        clearTimeout(filenameRefreshTimerRef.current);
        filenameRefreshTimerRef.current = null;
      }
    };
  }, [currentStatus, correcting, text]);

  const handleClear = () => {
    // Optimistic: clear the visible text immediately regardless of the round-trip, so
    // the button always visibly responds even if the backend call is slow or fails.
    setRawText('');
    setText('');
    setHistory([]);
    setPreviewThreshold(null);
    setSemanticFilename(null);
    invoke('clear_compose_text').catch((error) => {
      console.error('Failed to clear Compose text:', error);
    });
  };

  const handleDump = () => {
    invoke<ComposeDumpResult>('dump_compose_text', { text: previewText })
      .then((result) => {
        setRawText('');
        setText('');
        setHistory([]);
        setSemanticFilename(null);
        setDumpStatus(`Offloaded as ${result.fileName}`);
        setTimeout(() => setDumpStatus(null), 4000);
      })
      .catch((error) => {
        console.error('Failed to dump Compose text:', error);
        setDumpStatus(typeof error === 'string' ? error : 'Offload failed.');
        setTimeout(() => setDumpStatus(null), 4000);
      });
  };

  const handleOpenDictationsFolder = () => {
    invoke('open_dictations_folder').catch((error) => {
      console.error('Failed to open dictations folder:', error);
    });
  };

  const selectOffloadLocation = (path: string, remember = rememberOffloadLocation) => {
    invoke('set_dump_dir_override', { path, remember })
      .then(() => setDictationsDir(path))
      .catch((error) => {
        setDumpStatus(typeof error === 'string' ? error : 'Could not use that offload location.');
      });
  };

  const toggleRememberOffloadLocation = (remember: boolean) => {
    setRememberOffloadLocation(remember);
    if (dictationsDir) selectOffloadLocation(dictationsDir, remember);
  };

  const toggleRecording = () => {
    const isRecording = currentStatus === 'Recording';
    setRecordingCommandPending(true);
    invoke(isRecording ? 'stop_recording' : 'start_recording')
      .catch((error) => console.error(`Failed to ${isRecording ? 'stop' : 'start'} recording:`, error))
      .finally(() => setRecordingCommandPending(false));
  };

  const revertBatch = (batchId: number, keepStageCount: number) => {
    invoke('revert_compose_batch', { batchId, keepStageCount })
      .then(() => {
        invoke<ComposeState>('get_compose_text').then((state) => {
          setRawText(state.rawText);
          setText(state.text);
        });
        refreshHistory();
      })
      .catch((error) => {
        console.error('Failed to revert Compose batch:', error);
      });
  };

  const paneStyle = {
    padding: '18px 20px',
    borderRadius: tokens.radii.panel,
    border: '1px solid rgba(255, 255, 255, 0.08)',
    background: 'linear-gradient(145deg, rgba(255,255,255,0.035), rgba(255,255,255,0.012))',
    boxShadow: 'none',
    minHeight: 0,
    height: '100%',
    overflowY: 'scroll',
  };

  return (
    <div style={{ ...tabPanelStyle, height: '100%', minHeight: 0, overflow: 'hidden' }} key="compose">
      <style>{`
        @keyframes voxbridge-agent-spin { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }
        @keyframes voxbridge-agent-pulse { 0%, 100% { opacity: 0.55; } 50% { opacity: 1; } }
      `}</style>
      <div style={{ ...tabPanelPaddedStyle, position: 'relative', display: 'grid', gridTemplateRows: 'auto auto minmax(0, 1fr)', gap: '10px', paddingBottom: '14px', height: '100%', minHeight: 0 }}>
        <header style={{ display: 'flex', alignItems: 'center', gap: '12px', padding: '4px 2px' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: '5px', minWidth: 0, overflow: 'visible', flex: '1 1 auto' }}>
            <span style={{ flexShrink: 0, marginRight: '2px', color: tokens.colors.textMuted, fontSize: tokens.typography.sizeXs }}>
              Agents
            </span>
            {enabledAgents.map((agent, index) => (
              <div
                key={agent.id}
                style={{ position: 'relative', flexShrink: 0, paddingBottom: '8px', marginBottom: '-8px' }}
                onMouseEnter={() => setInspectedAgentId(agent.id)}
                onMouseLeave={() => setInspectedAgentId(null)}
              >
                <button
                  type="button"
                  onClick={() => {
                    selectAgentIndex(index);
                    onEditAgent(agent.id);
                  }}
                  onFocus={() => setInspectedAgentId(agent.id)}
                  onBlur={() => setInspectedAgentId(null)}
                  aria-describedby={inspectedAgentId === agent.id ? `agent-summary-${agent.id}` : undefined}
                  style={{ border: 0, borderRadius: '6px', background: index === clampedAgentIndex ? 'rgba(255,138,0,.12)' : 'transparent', color: index === clampedAgentIndex ? tokens.colors.accentSoft : tokens.colors.textSecondary, padding: '5px 7px', cursor: 'pointer', fontSize: '11px', fontWeight: 600 }}
                >
                  {correcting && activeAgent === (agentPresets.find((preset) => preset.id === agent.presetId)?.name || agent.name) && (
                    <IconRefresh size={13} style={{ marginRight: '5px', verticalAlign: '-2px', animation: 'voxbridge-agent-spin 1.1s linear infinite' }} />
                  )}
                  {agentPresets.find((preset) => preset.id === agent.presetId)?.name || agent.name}
                </button>
                {inspectedAgentId === agent.id && (
                  <div
                    id={`agent-summary-${agent.id}`}
                    role="tooltip"
                    style={{ position: 'absolute', top: '100%', left: 0, zIndex: 30, width: 'min(430px, 46vw)', padding: '12px', borderRadius: '10px', border: '1px solid rgba(255,184,0,.25)', background: tokens.colors.glassBgHeavy, boxShadow: tokens.shadows.lg, color: tokens.colors.textSecondary, fontSize: tokens.typography.sizeXs, lineHeight: 1.45 }}
                  >
                    <div style={{ display: 'flex', gap: '12px', marginBottom: '8px', color: tokens.colors.textMuted, flexWrap: 'wrap' }}>
                      <span style={{ display: 'inline-flex', alignItems: 'center', gap: '5px' }}>
                        Fidelity <strong style={{ color: tokens.colors.textPrimary }}>{agent.minFidelity.toFixed(2)}</strong>
                        <AgentHelp label="Fidelity" text="Minimum similarity required before this agent's result is accepted." />
                      </span>
                      <span style={{ display: 'inline-flex', alignItems: 'center', gap: '5px' }}>
                        Speed <strong style={{ color: tokens.colors.textPrimary }}>{agent.speed}</strong>
                        <AgentHelp label="Speed" text="Controls the processing and output budget available to this agent." />
                      </span>
                      <span style={{ display: 'inline-flex', alignItems: 'center', gap: '5px' }}>
                        Stage <strong style={{ color: tokens.colors.textPrimary }}>{agent.priority + 1}</strong>
                        <AgentHelp label="Stage" text="This agent's execution order in the refinement pipeline." />
                      </span>
                      <span style={{ display: 'inline-flex', alignItems: 'center', gap: '5px' }}>
                        History <strong style={{ color: tokens.colors.textPrimary }}>{agent.includeHistory ? `${agent.historyItems || 3} entries` : 'Off'}</strong>
                        <AgentHelp label="History" text="Recent saved History entries supplied as read-only context. Private mode always disables this." />
                      </span>
                    </div>
                    <div style={{ display: 'flex', alignItems: 'center', gap: '6px', marginBottom: '5px', color: tokens.colors.textMuted, fontSize: tokens.typography.sizeXs, fontWeight: 650 }}>
                      Prompt
                      <AgentHelp label="Prompt" text="Instructions this agent follows when refining the current transcript." />
                    </div>
                    <div style={{ color: tokens.colors.textPrimary, whiteSpace: 'pre-wrap', maxHeight: '180px', overflowY: 'auto' }}>{agent.prompt}</div>
                    <div style={{ marginTop: '8px', color: tokens.colors.accentSoft }}>Click to edit this agent</div>
                  </div>
                )}
              </div>
            ))}
          </div>
          {selectedAgent && (
            <label style={{ display: 'inline-flex', alignItems: 'center', gap: '7px', flexShrink: 0, color: tokens.colors.textMuted, fontSize: '11px' }}>
              Fidelity
              <input type="range" min={0} max={1} step={0.05} value={effectiveThreshold} onInput={(e: Event) => handleThresholdChange(Number((e.target as HTMLInputElement).value))} style={{ width: '100px' }} />
              <strong style={{ width: '30px', color: tokens.colors.textPrimary }}>{effectiveThreshold.toFixed(2)}</strong>
            </label>
          )}
          {selectedAgent && <Button variant="secondary" size="sm" onClick={applyThreshold} disabled={previewThreshold === null || isApplyingThreshold}>{isApplyingThreshold ? 'Applying…' : 'Apply'}</Button>}
          <div style={{ display: 'flex', alignItems: 'center', gap: '10px', flexShrink: 0 }}>
            <span style={{ display: 'inline-flex', alignItems: 'center', gap: '7px', color: tokens.colors.textMuted, fontSize: tokens.typography.sizeXs }}>
              <span style={{ width: '7px', height: '7px', borderRadius: '50%', background: statusColor }} />
              {currentStatus}
            </span>
            <Button
              variant={currentStatus === 'Recording' ? 'danger' : 'secondary'}
              size="md"
              onClick={toggleRecording}
              disabled={recordingCommandPending || (!enginesReady && currentStatus !== 'Recording') || (!['Ready', 'Recording'].includes(currentStatus))}
              title={enginesReady
                ? `${currentStatus === 'Recording' ? 'Stop recording' : 'Start recording'}${recordingShortcut ? ` · ${recordingShortcut}` : ''}`
                : 'Speech recognition and text refinement must be ready'}
            >
              {currentStatus === 'Recording' ? <IconPlayerStopFilled size={16} /> : <IconMicrophone size={16} />}
              {currentStatus === 'Recording' ? 'Stop recording' : 'Start recording'}
            </Button>
          </div>
        </header>

        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <span style={{ fontSize: tokens.typography.sizeXs, color: tokens.colors.textMuted, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            {dumpStatus
              ? dumpStatus
              : correcting
                ? `${activeAgent ? `${activeAgent} is working...` : 'Cleaning up...'}`
                : previewText
                  ? semanticFilename ?? `${generateFilenameSlug(previewText)}.txt`
                  : ''}
          </span>
          <div style={{ display: 'flex', gap: tokens.spacing.xs }}>
            <Button variant="ghost" size="sm" onClick={() => setHistoryOpen(!historyOpen)} title="Contribution history">
              <IconHistory size={14} />
            </Button>
            <Button variant="ghost" size="sm" onClick={() => onCopyToClipboard(previewText)} disabled={!previewText} title="Copy all">
              <IconCopy size={14} />
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={handleDump}
              disabled={!previewText}
              title={
                previewText && dictationsDir
                  ? `Offload to ${dictationsDir} and clear the workspace`
                  : 'Offload composed text and clear the workspace'
              }
            >
              <IconDownload size={14} /> Offload
            </Button>
            <select
              value={dictationsDir ?? ''}
              onChange={(event: Event) => selectOffloadLocation((event.currentTarget as HTMLSelectElement).value)}
              aria-label="Offload location"
              title="Offload location for this session"
              style={{ maxWidth: '210px', padding: '5px 8px', borderRadius: '7px', border: '1px solid rgba(255,255,255,.1)', background: tokens.colors.bgSecondary, color: tokens.colors.textSecondary, fontSize: '11px' }}
            >
              {offloadLocations.map((path) => <option key={path} value={path}>{path}</option>)}
            </select>
            <label title="Keep the selected offload location for the next application session." style={{ display: 'inline-flex', alignItems: 'center', gap: '5px', color: tokens.colors.textMuted, fontSize: '10px', whiteSpace: 'nowrap' }}>
              <input type="checkbox" checked={rememberOffloadLocation} onChange={(event: Event) => toggleRememberOffloadLocation((event.currentTarget as HTMLInputElement).checked)} />
              Remember
            </label>
            <Button variant="ghost" size="sm" onClick={handleOpenDictationsFolder} title={dictationsDir ? `Open ${dictationsDir}` : 'Open offload location'}>
              <IconFolderOpen size={14} />
            </Button>
            <Button variant="ghost" size="sm" onClick={handleClear} disabled={!text && !rawText} title="Clear">
              <IconTrash size={14} />
            </Button>
          </div>
        </div>

        <div className="compose-text-grid" style={{ display: 'grid', gridTemplateColumns: 'repeat(2, minmax(0, 1fr))', gap: '12px', height: '100%', minHeight: 0 }}>
          <section style={{ display: 'grid', gridTemplateRows: 'auto minmax(0, 1fr)', gap: '6px', minHeight: 0 }}>
            <div style={{ color: tokens.colors.textMuted, fontSize: tokens.typography.sizeXs }}>Raw transcript</div>
            <div
              ref={originalPaneRef}
              onScroll={(event: Event) => updateFollowState(event.currentTarget as HTMLDivElement, originalFollowsOutput)}
              style={paneStyle}
            >
              {rawText ? (
                <div style={{ color: tokens.colors.textSecondary, fontSize: tokens.typography.sizeMd, lineHeight: 1.7, whiteSpace: 'pre-wrap' }}>{rawText}</div>
              ) : null}
            </div>
          </section>

          <section style={{ display: 'grid', gridTemplateRows: 'auto minmax(0, 1fr)', gap: '6px', minHeight: 0 }}>
            <div style={{ color: tokens.colors.textMuted, fontSize: tokens.typography.sizeXs }}>Refined</div>
            <div
              ref={polishedPaneRef}
              onScroll={(event: Event) => updateFollowState(event.currentTarget as HTMLDivElement, polishedFollowsOutput)}
              style={paneStyle}
            >
              {previewText ? (
                <div style={{ color: tokens.colors.textPrimary, fontSize: tokens.typography.sizeMd, lineHeight: 1.7, whiteSpace: 'pre-wrap' }}>{previewText}</div>
              ) : null}
            </div>
          </section>
        </div>

        {historyOpen && (
          <div style={{ position: 'absolute', zIndex: 25, left: '24px', right: '24px', bottom: '14px', display: 'flex', flexDirection: 'column', gap: tokens.spacing.xs, maxHeight: '45%', overflowY: 'auto', padding: '12px', border: '1px solid rgba(255,184,0,.18)', borderRadius: tokens.radii.panel, background: 'rgba(12,11,10,.98)', boxShadow: tokens.shadows.lg }}>
            <div style={{ fontSize: tokens.typography.sizeXs, color: tokens.colors.textMuted }}>
              Contribution History
            </div>
            {history.length === 0 ? (
              <div style={{ fontSize: tokens.typography.sizeXs, color: tokens.colors.textMuted }}>Nothing corrected yet this session.</div>
            ) : (
              [...history].reverse().map((batch) => (
                <Card key={batch.id} style={{ padding: '10px 12px', borderRadius: '10px', border: '1px solid rgba(255, 255, 255, 0.08)', background: 'rgba(255, 255, 255, 0.02)', boxShadow: 'none' }}>
                  <button
                    type="button"
                    onClick={() => setExpandedBatchId(expandedBatchId === batch.id ? null : batch.id)}
                    style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', width: '100%', background: 'none', border: 'none', cursor: 'pointer', color: tokens.colors.textPrimary, padding: 0 }}
                  >
                    <span style={{ fontSize: tokens.typography.sizeXs, textAlign: 'left', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flex: 1, marginRight: tokens.spacing.sm }}>
                      {batch.rawText}
                    </span>
                    {expandedBatchId === batch.id ? <IconChevronUp size={14} /> : <IconChevronDown size={14} />}
                  </button>

                  {expandedBatchId === batch.id && (
                    <div style={{ display: 'flex', flexDirection: 'column', gap: tokens.spacing.xs, marginTop: tokens.spacing.sm }}>
                      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                        <span style={{ fontSize: '11px', color: tokens.colors.textMuted }}>Raw</span>
                        <Button variant="ghost" size="sm" onClick={() => revertBatch(batch.id, 0)}>Revert to raw</Button>
                      </div>
                      <div style={{ fontSize: tokens.typography.sizeXs, color: tokens.colors.textSecondary, whiteSpace: 'pre-wrap' }}>{batch.rawText}</div>

                      {batch.stages.map((stage, i) => (
                        <div key={i} style={{ borderTop: '1px solid rgba(255, 255, 255, 0.06)', paddingTop: tokens.spacing.xs }}>
                          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                            <span style={{ fontSize: '11px', color: stage.accepted ? tokens.colors.accentPrimary : tokens.colors.error }}>
                              {stage.agentName} {stage.accepted ? '' : '(rejected)'}
                            </span>
                            <Button variant="ghost" size="sm" onClick={() => revertBatch(batch.id, i + 1)}>Revert to here</Button>
                          </div>
                          <div style={{ fontSize: tokens.typography.sizeXs, color: stage.accepted ? tokens.colors.textSecondary : tokens.colors.textMuted, whiteSpace: 'pre-wrap' }}>
                            {stage.text}
                          </div>
                          {stage.note && (
                            <div style={{ fontSize: '10px', color: tokens.colors.textMuted, fontStyle: 'italic' }}>{stage.note}</div>
                          )}
                        </div>
                      ))}
                    </div>
                  )}
                </Card>
              ))
            )}
          </div>
        )}
      </div>
    </div>
  );
}
