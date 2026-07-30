import { useEffect, useState } from 'preact/hooks';
import { invoke } from '@tauri-apps/api/core';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { IconRefresh, IconRocket } from '@tabler/icons-preact';
import { ConfigField } from '../components/ConfigField.tsx';
import { Switch } from '../components/Switch.tsx';
import { CollapsibleSection } from '../components/CollapsibleSection.tsx';
import { Button } from '../components/Button.tsx';
import { NumberField } from '../components/NumberField.tsx';
import { MicSetupPanel } from '../components/MicSetupPanel.tsx';
import { ModelSelectionPanel } from '../components/ModelSelectionPanel.tsx';
import { SelectField } from '../components/SelectField.tsx';
import { helperTextStyle, inputBaseStyle, selectWrapperStyle, tabPanelContentStyle, tabPanelStyle } from '../theme/ui-primitives.ts';
import { tokens } from '../design-tokens.ts';

interface AudioDevice {
  id: string;
  label: string;
}

interface OllamaModelInfo {
  name: string;
  sizeBytes: number;
  parameterSize: string;
  quantizationLevel: string;
}

interface EmbeddedModelOption {
  id: string;
  name: string;
  description: string;
  filePath: string;
  downloadBytes: number;
  installed: boolean;
  recommended: boolean;
  fitsGraphicsMemory: boolean | null;
  estimatedCombinedVramBytes: number;
  graphicsMemoryNote: string;
}

interface ComposeAgent {
  id: string;
  name: string;
  prompt: string;
  priority: number;
  speed: 'low' | 'medium' | 'high';
  enabled: boolean;
  presetId?: string | null;
  minFidelity: number;
  includeHistory: boolean;
  historyItems: number;
}

interface AgentPreset {
  id: string;
  name: string;
  prompt: string;
  builtin: boolean;
  minFidelity: number;
}

function FieldLabel({ label, help }: { label: string; help?: string }) {
  const [open, setOpen] = useState(false);
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: '6px', color: tokens.colors.textMuted, fontSize: tokens.typography.sizeXs, fontWeight: 600 }}>
      {label}
      {help && (
        <span style={{ position: 'relative', display: 'inline-flex' }} onMouseEnter={() => setOpen(true)} onMouseLeave={() => setOpen(false)}>
          <button type="button" onFocus={() => setOpen(true)} onBlur={() => setOpen(false)} aria-label={`${label}: ${help}`} style={{ width: '16px', height: '16px', padding: 0, borderRadius: '50%', border: '1px solid rgba(255,255,255,.18)', background: 'transparent', color: tokens.colors.textMuted, fontSize: '10px', lineHeight: '14px', cursor: 'help' }}>?</button>
          {open && (
            <span role="tooltip" style={{ position: 'absolute', left: 0, top: '21px', zIndex: 60, width: 'min(320px, 42vw)', padding: '9px 11px', borderRadius: '7px', border: '1px solid rgba(255,138,0,.22)', background: tokens.colors.glassBgHeavy, boxShadow: tokens.shadows.lg, color: tokens.colors.textSecondary, fontSize: tokens.typography.sizeXs, fontWeight: 450, lineHeight: 1.45 }}>
              {help}
            </span>
          )}
        </span>
      )}
    </span>
  );
}

function makeAgentId(): string {
  return `agent-${Date.now()}-${Math.floor(Math.random() * 10000)}`;
}

// Discrete steps for the Compose "Correction Context" slider. 0 means "Everything" -
// never batch-trigger on sentence count, only once the user actually pauses - so it has
// to sit at one end of the slider rather than in numeric order.
const CONTEXT_STEPS: { value: number; label: string }[] = [
  { value: 1, label: '1 sentence' },
  { value: 2, label: '2 sentences' },
  { value: 3, label: '3 sentences' },
  { value: 5, label: '5 sentences' },
  { value: 8, label: '~1 paragraph' },
  { value: 15, label: '~3 paragraphs' },
  { value: 0, label: 'Everything' },
];

const HISTORY_RETENTION_STEPS: { value: number; label: string }[] = [
  { value: 1, label: '1 day' },
  { value: 7, label: '7 days' },
  { value: 14, label: '14 days' },
  { value: 30, label: '30 days' },
  { value: 90, label: '90 days' },
  { value: 180, label: '180 days' },
  { value: 365, label: '1 year' },
  { value: 0, label: 'Forever' },
];

function historyRetentionIndex(value: number): number {
  const index = HISTORY_RETENTION_STEPS.findIndex((step) => step.value === value);
  return index >= 0 ? index : HISTORY_RETENTION_STEPS.length - 1;
}

function contextStepIndex(value: number): number {
  const index = CONTEXT_STEPS.findIndex((step) => step.value === value);
  if (index >= 0) return index;
  // Unrecognized/custom value (e.g. an older config) - snap to the closest non-zero step.
  let closest = 0;
  let closestDiff = Infinity;
  CONTEXT_STEPS.forEach((step, i) => {
    if (step.value === 0) return;
    const diff = Math.abs(step.value - value);
    if (diff < closestDiff) {
      closest = i;
      closestDiff = diff;
    }
  });
  return closest;
}

function contextStepLabel(value: number): string {
  return CONTEXT_STEPS[contextStepIndex(value)].label;
}

interface ConfigPageProps {
  config: {
    transcription_mode: 'API' | 'Local';
    local_model_size: string;
    local_engine: string;
    hotkey: string;
    hotkey_mode: 'sticky' | 'hold' | 'continuous';
    continuous_silence_ms: number;
    custom_vocabulary: string;
    openai_api_key: string;
    api_url: string;
    api_model: string;
    copy_on_typewriter: boolean;
    output_method: 'Typewriter' | 'Clipboard' | 'Compose';
    compose_backend: 'embedded' | 'ollama_remote';
    compose_use_gpu: boolean;
    compose_model_path: string;
    compose_ollama_url: string;
    compose_ollama_model: string;
    compose_context_sentences: number;
    compose_edit_latency: 'low' | 'medium' | 'high';
    compose_pause_only: boolean;
    audio_device: string | null;
    input_sensitivity: number;
    typing_speed_interval: number;
    key_press_duration_ms: number;
    pixels_from_bottom: number;
    debug_mode: boolean;
    enable_gpu: boolean;
    enable_recording_logs: boolean;
    recording_log_retention_days: number;
    enable_history: boolean;
    history_retention_days: number;
    offload_locations: string[];
    default_offload_location: string;
    remember_offload_location: boolean;
    last_offload_location: string;
    include_recognized_in_offload: boolean;
    include_recording_in_offload: boolean;
    post_roll_ms: number;
    check_for_updates_on_startup: boolean;
  };
  activeConfigSection: string | null;
  setActiveConfigSection: (value: string | null) => void;
  agentSettingsTargetId: string | null;
  isModelLoading: boolean;
  availableModels: any[];
  modelStatus: Record<string, boolean>;
  downloadProgress: number;
  isDownloading: boolean;
  portalVersion: number;
  isSystemManagedShortcut: boolean;
  hotkeyBindingState: { bound: boolean; active_trigger?: string } | null;
  isApplyingHotkey: boolean;
  availableMics: AudioDevice[];
  micTestStatus: 'idle' | 'recording' | 'playing' | 'processing';
  micVolume: number;
  updateConfig: (key: string, value: any) => void;
  downloadModel: (size: string) => void;
  loadModels: () => void;
  loadMics: () => void;
  handleConfigureHotkey: () => void;
  setShowModelGuide: (show: boolean) => void;
  startMicTest: () => void;
  stopMicTest: () => void;
  stopMicPlayback: () => void;
  openDebugFolder: () => void;
  onReopenInitialSetup: () => void;
  onFactoryReset: () => void;
  checkingUpdates: boolean;
  onCheckForUpdates: () => void;
  autostartEnabled: boolean;
  onToggleAutostart: (enabled: boolean) => void;
}


export function ConfigPage(props: ConfigPageProps) {
  const {
    config,
    activeConfigSection,
    setActiveConfigSection,
    agentSettingsTargetId,
    isModelLoading,
    availableModels,
    modelStatus,
    downloadProgress,
    isDownloading,
    portalVersion,
    isSystemManagedShortcut,
    hotkeyBindingState,
    isApplyingHotkey,
    availableMics,
    micTestStatus,
    micVolume,
    updateConfig,
    downloadModel,
    loadModels,
    loadMics,
    handleConfigureHotkey,
    setShowModelGuide,
    startMicTest,
    stopMicTest,
    stopMicPlayback,
    openDebugFolder,
    onReopenInitialSetup,
    onFactoryReset,
    checkingUpdates,
    onCheckForUpdates,
    autostartEnabled,
    onToggleAutostart,
  } = props;

  interface GpuVramCheck {
    vulkan_runtime_available: boolean;
    gpu_detected: boolean;
    adapter_name: string | null;
    available_vram_bytes: number | null;
    dedicated_vram_bytes: number | null;
    required_estimate_bytes: number;
    usage_percent: number | null;
    supported: boolean;
    reason: string | null;
  }

  const [gpuVramCheck, setGpuVramCheck] = useState<GpuVramCheck | null>(null);

  useEffect(() => {
    if (config.transcription_mode !== 'Local' || !config.local_model_size) {
      setGpuVramCheck(null);
      return;
    }
    let cancelled = false;
    invoke<GpuVramCheck>('check_gpu_vram', { modelSize: config.local_model_size })
      .then((result) => {
        if (!cancelled) setGpuVramCheck(result);
      })
      .catch(() => {
        if (!cancelled) setGpuVramCheck(null);
      });
    return () => {
      cancelled = true;
    };
  }, [config.transcription_mode, config.local_model_size]);


  const [composeAgents, setComposeAgents] = useState<ComposeAgent[]>([]);
  const [expandedAgentId, setExpandedAgentId] = useState<string | null>(null);
  const [agentPresets, setAgentPresets] = useState<AgentPreset[]>([]);
  const [savingPresetForAgentId, setSavingPresetForAgentId] = useState<string | null>(null);
  const [presetNameDraft, setPresetNameDraft] = useState('');
  const [recordingLogStatus, setRecordingLogStatus] = useState<string | null>(null);

  useEffect(() => {
    if (!agentSettingsTargetId || !composeAgents.some((agent) => agent.id === agentSettingsTargetId)) return;
    setExpandedAgentId(agentSettingsTargetId);
    window.setTimeout(() => {
      document.getElementById(`agent-editor-${agentSettingsTargetId}`)?.scrollIntoView({ behavior: 'smooth', block: 'center' });
    }, 50);
  }, [agentSettingsTargetId, composeAgents]);

  useEffect(() => {
    invoke<ComposeAgent[]>('get_compose_agents')
      .then(setComposeAgents)
      .catch(() => {});
    refreshAgentPresets();
  }, []);

  const refreshAgentPresets = () => {
    invoke<AgentPreset[]>('get_agent_presets')
      .then(setAgentPresets)
      .catch(() => {});
  };

  const [dumpDirOverride, setDumpDirOverride] = useState<string | null>(null);
  const [dumpDirDraft, setDumpDirDraft] = useState('');
  const [defaultDumpDir, setDefaultDumpDir] = useState<string | null>(null);
  const [dumpDirStatus, setDumpDirStatus] = useState<string | null>(null);

  useEffect(() => {
    invoke<string | null>('get_dump_dir_override')
      .then((dir) => {
        setDumpDirOverride(dir);
        setDumpDirDraft(dir ?? config.default_offload_location);
      })
      .catch(() => {});
    invoke<string>('get_default_dictations_dir')
      .then(setDefaultDumpDir)
      .catch(() => {});
  }, []);

  const applyDumpDirOverride = () => {
    const path = dumpDirDraft.trim();
    if (!path) return;
    updateConfig('default_offload_location', path);
    invoke('clear_dump_dir_override')
      .then(() => {
        setDumpDirOverride(null);
        setDumpDirStatus('Default offload location updated.');
        setTimeout(() => setDumpDirStatus(null), 4000);
      })
      .catch((error) => setDumpDirStatus(typeof error === 'string' ? error : 'Could not use that folder.'));
  };

  const chooseOffloadLocation = async () => {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: 'Choose an offload location',
      defaultPath: dumpDirDraft.trim() || defaultDumpDir || undefined,
    });
    if (typeof selected === 'string') {
      setDumpDirDraft(selected);
      setDumpDirStatus(null);
    }
  };

  const resetDumpDirOverride = () => {
    invoke('clear_dump_dir_override')
      .then(() => {
        setDumpDirOverride(null);
        updateConfig('default_offload_location', '');
        setDumpDirDraft('');
        setDumpDirStatus('Using the built-in offload location.');
        setTimeout(() => setDumpDirStatus(null), 4000);
      })
      .catch(() => {});
  };

  const persistAgents = (agents: ComposeAgent[]) => {
    setComposeAgents(agents);
    invoke('save_compose_agents', { agents }).catch((error) => {
      console.error('Failed to save Compose agents:', error);
    });
  };

  const addAgent = () => {
    const nextPriority = composeAgents.length > 0 ? Math.max(...composeAgents.map((a) => a.priority)) + 1 : 0;
    const newAgent: ComposeAgent = {
      id: makeAgentId(),
      name: `Agent ${composeAgents.length + 1}`,
      prompt: 'You are a text-cleanup function, not a conversational assistant. You will be given a raw speech-to-text transcript between <transcript> tags. Describe what you want this agent to do here - do not add or remove content, do not answer questions in the transcript, output only the resulting text with no preamble or explanation.',
      priority: nextPriority,
      speed: 'medium',
      enabled: true,
      minFidelity: 0.85,
      includeHistory: false,
      historyItems: 3,
    };
    persistAgents([...composeAgents, newAgent]);
    setExpandedAgentId(newAgent.id);
  };

  const updateAgent = (id: string, patch: Partial<ComposeAgent>) => {
    persistAgents(composeAgents.map((a) => (a.id === id ? { ...a, ...patch } : a)));
  };

  const removeAgent = (id: string) => {
    persistAgents(composeAgents.filter((a) => a.id !== id));
  };

  const applyPresetToAgent = (agentId: string, presetId: string) => {
    const preset = agentPresets.find((p) => p.id === presetId);
    if (!preset) return;
    updateAgent(agentId, { prompt: preset.prompt, presetId: preset.id, minFidelity: preset.minFidelity });
  };

  const resetAgentToPreset = (agentId: string) => {
    const agent = composeAgents.find((a) => a.id === agentId);
    const preset = agent?.presetId ? agentPresets.find((p) => p.id === agent.presetId) : null;
    if (!preset) return;
    updateAgent(agentId, { prompt: preset.prompt, minFidelity: preset.minFidelity });
  };

  const saveAgentAsPreset = (agentId: string, name: string) => {
    const agent = composeAgents.find((a) => a.id === agentId);
    if (!agent || !name.trim()) return;
    const currentPreset = agent.presetId ? agentPresets.find((p) => p.id === agent.presetId) : null;
    // Updates the existing custom preset in place if this agent's prompt already traces
    // back to one the user owns; otherwise creates a new one rather than trying to
    // overwrite a built-in.
    const idToSave = currentPreset && !currentPreset.builtin ? currentPreset.id : '';
    invoke<AgentPreset>('save_agent_preset', { preset: { id: idToSave, name: name.trim(), prompt: agent.prompt, builtin: false, minFidelity: agent.minFidelity } })
      .then((saved) => {
        updateAgent(agentId, { presetId: saved.id });
        refreshAgentPresets();
        setSavingPresetForAgentId(null);
        setPresetNameDraft('');
      })
      .catch((error) => {
        console.error('Failed to save agent preset:', error);
      });
  };

  const moveAgent = (id: string, direction: -1 | 1) => {
    const sorted = [...composeAgents].sort((a, b) => a.priority - b.priority);
    const index = sorted.findIndex((a) => a.id === id);
    const swapWith = index + direction;
    if (index < 0 || swapWith < 0 || swapWith >= sorted.length) return;
    const a = sorted[index];
    const b = sorted[swapWith];
    persistAgents(
      composeAgents.map((agent) => {
        if (agent.id === a.id) return { ...agent, priority: b.priority };
        if (agent.id === b.id) return { ...agent, priority: a.priority };
        return agent;
      }),
    );
  };

  useEffect(() => {
    if (!activeConfigSection) setActiveConfigSection('general');
  }, [activeConfigSection, setActiveConfigSection]);

  const [ollamaModels, setOllamaModels] = useState<OllamaModelInfo[]>([]);
  const [isDetectingOllamaModels, setIsDetectingOllamaModels] = useState(false);
  const [ollamaDetectError, setOllamaDetectError] = useState<string | null>(null);
  const [embeddedModels, setEmbeddedModels] = useState<EmbeddedModelOption[]>([]);
  const [selectedEmbeddedModelId, setSelectedEmbeddedModelId] = useState<string>('custom');
  const [embeddedModelBusy, setEmbeddedModelBusy] = useState(false);
  const [embeddedModelError, setEmbeddedModelError] = useState<string | null>(null);

  const refreshEmbeddedModels = () => {
    invoke<EmbeddedModelOption[]>('list_embedded_compose_models')
      .then((models) => {
        setEmbeddedModels(models);
        const current = models.find((model) =>
          model.filePath.toLocaleLowerCase() === config.compose_model_path.toLocaleLowerCase());
        setSelectedEmbeddedModelId(current?.id ?? 'custom');
      })
      .catch((error) => setEmbeddedModelError(String(error)));
  };

  useEffect(() => {
    refreshEmbeddedModels();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [config.local_model_size, config.enable_gpu, config.compose_use_gpu]);

  const chooseEmbeddedModel = (modelId: string) => {
    setSelectedEmbeddedModelId(modelId);
    setEmbeddedModelError(null);
    const model = embeddedModels.find((candidate) => candidate.id === modelId);
    if (model?.fitsGraphicsMemory === false) {
      setEmbeddedModelError(`Selection blocked: ${model.graphicsMemoryNote}`);
      return;
    }
    if (model?.installed) updateConfig('compose_model_path', model.filePath);
  };

  const downloadSelectedEmbeddedModel = () => {
    const model = embeddedModels.find((candidate) => candidate.id === selectedEmbeddedModelId);
    if (!model) return;
    setEmbeddedModelBusy(true);
    setEmbeddedModelError(null);
    invoke<string>('download_embedded_compose_model', { modelId: model.id })
      .then((path) => {
        updateConfig('compose_model_path', path);
        refreshEmbeddedModels();
      })
      .catch((error) => setEmbeddedModelError(String(error)))
      .finally(() => setEmbeddedModelBusy(false));
  };

  const chooseCustomEmbeddedModel = async () => {
    const selected = await openDialog({
      multiple: false,
      directory: false,
      filters: [{ name: 'GGUF model', extensions: ['gguf'] }],
    });
    if (typeof selected === 'string') {
      setSelectedEmbeddedModelId('custom');
      updateConfig('compose_model_path', selected);
    }
  };

  const detectOllamaModels = () => {
    setIsDetectingOllamaModels(true);
    setOllamaDetectError(null);
    invoke<OllamaModelInfo[]>('list_ollama_models', { baseUrl: config.compose_ollama_url })
      .then((models) => {
        setOllamaModels(models);
        if (models.length === 0) {
          setOllamaDetectError('Connected, but no models found on that instance.');
        }
      })
      .catch((error) => {
        setOllamaModels([]);
        setOllamaDetectError(String(error));
      })
      .finally(() => setIsDetectingOllamaModels(false));
  };

  // ConfigPage remounts every time the user navigates away and back (Settings tab), which
  // wipes the local `ollamaModels` list - re-detect automatically so a previously selected
  // remote model doesn't appear blank just because nothing has re-populated its option yet.
  useEffect(() => {
    if (config.compose_backend === 'ollama_remote' && config.compose_ollama_url) {
      detectOllamaModels();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div style={{ ...tabPanelStyle, overflow: 'hidden', padding: 0 }} key="settings">
      <div style={{ display: 'grid', gridTemplateColumns: '190px minmax(0, 1fr)', height: '100%', minHeight: 0 }}>
        <aside style={{ padding: '14px 10px', borderRight: '1px solid rgba(255,255,255,.08)', background: 'rgba(12,12,11,.58)' }}>
          {[
            ['general', 'General'],
            ['compose', 'Text refinement'],
            ['agents', 'Agents'],
            ['transcription', 'Speech recognition'],
            ['audio', 'Audio'],
            ['logs', 'Data & logs'],
          ].map(([id, label]) => (
            <button
              key={id}
              type="button"
              onClick={() => setActiveConfigSection(id)}
              style={{
                display: 'block',
                width: '100%',
                padding: '9px 11px',
                border: 0,
                borderRadius: '7px',
                background: activeConfigSection === id ? 'rgba(255,138,0,.11)' : 'transparent',
                color: activeConfigSection === id ? tokens.colors.accentSoft : tokens.colors.textSecondary,
                fontSize: tokens.typography.sizeSm,
                fontWeight: activeConfigSection === id ? 650 : 500,
                textAlign: 'left',
                cursor: 'pointer',
              }}
            >
              {label}
            </button>
          ))}
        </aside>
        <div style={{ ...tabPanelContentStyle, maxWidth: '100%', margin: 0, minHeight: 0, overflowY: 'auto', paddingTop: '14px' }}>
        <CollapsibleSection title="General" isOpen={activeConfigSection === 'general'} onToggle={() => setActiveConfigSection(activeConfigSection === 'general' ? null : 'general')}>
          <ConfigField label="Offload locations" description="Set the default destination and maintain optional locations available from the Compose toolbar. A launch argument can still override the location for one session.">
            <div style={{ display: 'flex', flexDirection: 'column', gap: tokens.spacing.sm, width: '100%' }}>
              <div style={{ display: 'flex', gap: tokens.spacing.sm, width: '100%', alignItems: 'center' }}>
                <input
                  style={{ ...inputBaseStyle, flex: 1 }}
                  type="text"
                  value={dumpDirDraft}
                  readOnly
                  placeholder={defaultDumpDir ?? 'Documents\\VoxBridge Compose Offloads'}
                />
                <Button variant="secondary" size="sm" onClick={() => void chooseOffloadLocation()}>
                  Choose folder
                </Button>
                <Button variant="secondary" size="sm" onClick={applyDumpDirOverride} disabled={!dumpDirDraft.trim() || dumpDirDraft.trim() === config.default_offload_location}>
                  Set default
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    const path = dumpDirDraft.trim();
                    if (!path || config.offload_locations.includes(path)) return;
                    updateConfig('offload_locations', [...config.offload_locations, path]);
                    setDumpDirStatus('Saved to offload locations.');
                  }}
                  disabled={!dumpDirDraft.trim() || config.offload_locations.includes(dumpDirDraft.trim())}
                >
                  Save location
                </Button>
                <Button variant="ghost" size="sm" onClick={resetDumpDirOverride} disabled={!dumpDirOverride}>Reset</Button>
              </div>
              {config.offload_locations.map((path) => (
                <div key={path} style={{ display: 'flex', alignItems: 'center', gap: '8px', color: tokens.colors.textSecondary, fontSize: tokens.typography.sizeXs }}>
                  <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{path}</span>
                  <button type="button" onClick={() => updateConfig('offload_locations', config.offload_locations.filter((location) => location !== path))} style={{ border: 0, background: 'transparent', color: tokens.colors.textMuted, cursor: 'pointer', fontSize: '11px' }}>Remove</button>
                </div>
              ))}
              {dumpDirStatus && <div style={helperTextStyle}>{dumpDirStatus}</div>}
            </div>
          </ConfigField>

          <ConfigField label="Offload contents" description="Choose which source material is saved with the refined result. Audio is copied beside the text file only when recording logs and history are enabled and a source recording exists.">
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: tokens.spacing.md, flexWrap: 'wrap', width: '100%' }}>
              <label style={{ display: 'inline-flex', alignItems: 'center', gap: '8px', color: tokens.colors.textSecondary, fontSize: tokens.typography.sizeXs }}>
                <Switch checked={config.include_recognized_in_offload} onChange={(checked) => updateConfig('include_recognized_in_offload', checked)} />
                Include raw transcript
              </label>
              <label style={{ display: 'inline-flex', alignItems: 'center', gap: '8px', color: config.enable_recording_logs ? tokens.colors.textSecondary : tokens.colors.textMuted, fontSize: tokens.typography.sizeXs }}>
                <Switch checked={config.include_recording_in_offload} disabled={!config.enable_recording_logs || !config.enable_history} onChange={(checked) => updateConfig('include_recording_in_offload', checked)} />
                Include source audio
              </label>
            </div>
          </ConfigField>

          <ConfigField label="Application" description="Control startup behavior or rerun the guided checks for audio, shortcuts, and local models.">
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: tokens.spacing.md, width: '100%', flexWrap: 'wrap' }}>
              <label style={{ display: 'inline-flex', alignItems: 'center', gap: '8px', color: tokens.colors.textSecondary, fontSize: tokens.typography.sizeXs }}>
                <Switch checked={autostartEnabled} onChange={onToggleAutostart} />
                Launch at system startup
              </label>
              <Button variant="secondary" size="sm" onClick={onReopenInitialSetup}>Run setup checks</Button>
            </div>
          </ConfigField>

          <ConfigField label="History" description="Turning this off activates Private mode. While active, VoxBridge Compose does not save recognized or refined text, recording audio, or diagnostic session logs. Existing files are not deleted.">
            <div style={{ display: 'flex', flexDirection: 'column', gap: '10px', width: '100%' }}>
              <Switch checked={config.enable_history} onChange={(checked) => updateConfig('enable_history', checked)} />
              {config.enable_history && (
                <div style={{ display: 'flex', flexDirection: 'column', gap: '6px' }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', color: tokens.colors.textSecondary, fontSize: tokens.typography.sizeXs }}>
                    <span>Keep for</span>
                    <strong style={{ color: tokens.colors.textPrimary }}>
                      {HISTORY_RETENTION_STEPS[historyRetentionIndex(config.history_retention_days)].label}
                    </strong>
                  </div>
                  <input
                    type="range"
                    min={0}
                    max={HISTORY_RETENTION_STEPS.length - 1}
                    step={1}
                    value={historyRetentionIndex(config.history_retention_days)}
                    onInput={(event: Event) => {
                      const index = Number((event.target as HTMLInputElement).value);
                      updateConfig('history_retention_days', HISTORY_RETENTION_STEPS[index].value);
                    }}
                    aria-label="History retention"
                    style={{ width: '100%' }}
                  />
                  <div style={{ display: 'flex', justifyContent: 'space-between', color: tokens.colors.textMuted, fontSize: '10px' }}>
                    <span>1 day</span>
                    <span>Forever</span>
                  </div>
                </div>
              )}
            </div>
          </ConfigField>

          <ConfigField label="Updates" description="Check the release source after launch or manually view the latest version, changelog, and repository link. Nothing is installed automatically.">
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: tokens.spacing.md, flexWrap: 'wrap', width: '100%' }}>
              <label style={{ display: 'inline-flex', alignItems: 'center', gap: '8px', color: tokens.colors.textSecondary, fontSize: tokens.typography.sizeXs }}>
                <Switch
                  checked={config.check_for_updates_on_startup}
                  onChange={(checked) => updateConfig('check_for_updates_on_startup', checked)}
                />
                Check at startup
              </label>
              <Button variant="secondary" size="sm" onClick={onCheckForUpdates} disabled={checkingUpdates}>
                {checkingUpdates ? 'Checking...' : 'Check for updates'}
              </Button>
            </div>
          </ConfigField>

          <ConfigField label="Reset application" description="Deletes downloaded speech-recognition models, custom agents and profiles, recordings, logs, history, saved offload locations, and local settings. External text-refinement model files are not deleted.">
            <Button variant="danger" size="sm" onClick={onFactoryReset}>Reset local data</Button>
          </ConfigField>

        </CollapsibleSection>

        {config.output_method === 'Compose' && (
          <CollapsibleSection title={activeConfigSection === 'agents' ? 'Agents' : 'Text refinement'} isOpen={activeConfigSection === 'compose' || activeConfigSection === 'agents'} onToggle={() => setActiveConfigSection(null)}>
            {activeConfigSection === 'compose' && (<>
            <ConfigField label="Refinement model provider" description="Embedded runs the text-refinement model inside VoxBridge. Ollama uses the selected Ollama service and model instead.">
              <div style={selectWrapperStyle}>
                <SelectField
                  value={config.compose_backend}
                  onChange={(val) => updateConfig('compose_backend', val)}
                  searchable={false}
                  ariaLabel="Text-refinement model provider"
                  options={[
                    { value: 'embedded', label: 'Embedded (local model file)' },
                    { value: 'ollama_remote', label: 'Remote Ollama' },
                  ]}
                />
              </div>
            </ConfigField>

            {config.compose_backend === 'embedded' ? (
              <>
                <ConfigField label="Refinement graphics acceleration" description="Uses VoxBridge graphics acceleration for the embedded text-refinement model. Speech recognition has a separate setting. This does not control Ollama.">
                  <Switch checked={config.compose_use_gpu} onChange={(checked) => updateConfig('compose_use_gpu', checked)} />
                </ConfigField>
                <ConfigField label="Embedded refinement model" description="Choose a managed local model. Download and selection are blocked when the combined speech-recognition and refinement estimate exceeds dedicated graphics memory. Custom GGUF files remain available for advanced use.">
                  <div style={{ display: 'flex', flexDirection: 'column', gap: tokens.spacing.sm, width: '100%' }}>
                    <div style={selectWrapperStyle}>
                      <SelectField
                        value={selectedEmbeddedModelId}
                        onChange={chooseEmbeddedModel}
                        searchable={false}
                        ariaLabel="Embedded refinement model"
                        options={[
                          ...embeddedModels.map((model) => ({
                            value: model.id,
                            label: `${model.name}${model.recommended ? ' · Recommended' : ''} · ${Math.round(model.downloadBytes / 1024 / 1024)} MB${model.installed ? ' · Installed' : ''}`,
                          })),
                          { value: 'custom', label: 'Custom GGUF file' },
                        ]}
                      />
                      {selectedEmbeddedModelId === 'custom' ? (
                        <Button variant="secondary" size="sm" onClick={chooseCustomEmbeddedModel}>Browse</Button>
                      ) : (() => {
                        const selected = embeddedModels.find((model) => model.id === selectedEmbeddedModelId);
                        if (!selected || selected.installed) return null;
                        return (
                          <Button
                            variant="secondary"
                            size="sm"
                            onClick={downloadSelectedEmbeddedModel}
                            disabled={embeddedModelBusy || selected.fitsGraphicsMemory === false}
                          >
                            {embeddedModelBusy ? 'Downloading...' : 'Download'}
                          </Button>
                        );
                      })()}
                    </div>
                    {(() => {
                      const selected = embeddedModels.find((model) => model.id === selectedEmbeddedModelId);
                      if (!selected) {
                        return config.compose_model_path
                          ? <div style={helperTextStyle}>{config.compose_model_path}</div>
                          : null;
                      }
                      return (
                        <div style={{ ...helperTextStyle, color: selected.fitsGraphicsMemory === false ? tokens.colors.error : tokens.colors.textMuted }}>
                          {selected.description} {selected.graphicsMemoryNote}
                        </div>
                      );
                    })()}
                    {embeddedModelError && (
                      <div style={{ ...helperTextStyle, color: tokens.colors.error }}>{embeddedModelError}</div>
                    )}
                  </div>
                </ConfigField>
              </>
            ) : (
              <>
                <ConfigField label="Ollama address" description="Network address of the Ollama instance to use.">
                  <input
                    style={inputBaseStyle}
                    type="text"
                    value={config.compose_ollama_url}
                    onChange={(e: Event) => updateConfig('compose_ollama_url', (e.target as HTMLInputElement).value)}
                    placeholder="http://localhost:11434"
                  />
                </ConfigField>
                <ConfigField label="Model" description="Model to use from that Ollama instance's own list.">
                  <div style={{ display: 'flex', flexDirection: 'column', gap: tokens.spacing.sm, width: '100%' }}>
                    <div style={selectWrapperStyle}>
                      <SelectField
                        value={config.compose_ollama_model}
                        onChange={(val) => updateConfig('compose_ollama_model', val)}
                        searchable={false}
                        ariaLabel="Ollama model"
                        placeholder={ollamaModels.length > 0 ? 'Select a model' : 'Detect models first'}
                        options={[
                          // The saved selection may not be in the freshly-fetched list yet
                          // (detection is still in flight, or failed) - show it plainly
                          // rather than let the select appear empty/unselected.
                          ...(config.compose_ollama_model && !ollamaModels.some((m) => m.name === config.compose_ollama_model)
                            ? [{ value: config.compose_ollama_model, label: config.compose_ollama_model }]
                            : []),
                          ...ollamaModels.map((m) => ({
                            value: m.name,
                            label: `${m.name} (${[m.parameterSize, m.quantizationLevel].filter(Boolean).join(', ')}${m.parameterSize || m.quantizationLevel ? ', ' : ''}${Math.round(m.sizeBytes / 1024 / 1024)}MB)`,
                          })),
                        ]}
                      />
                      <Button variant="icon" onClick={detectOllamaModels} disabled={isDetectingOllamaModels} title="Detect models">
                        <IconRefresh size={20} />
                      </Button>
                    </div>
                    {ollamaDetectError && (
                      <div style={{ ...helperTextStyle, color: tokens.colors.error }}>{ollamaDetectError}</div>
                    )}
                  </div>
                </ConfigField>
              </>
            )}

            </>)}
            {activeConfigSection === 'agents' && (<>
            <ConfigField
              label="Agent pipeline"
              description="Processes each batch through enabled agents in priority order. Every agent receives the previous agent's accepted output."
            >
              <div style={{ display: 'flex', flexDirection: 'column', gap: tokens.spacing.sm, width: '100%' }}>
                <div style={{ display: 'flex', alignItems: 'stretch', gap: '4px', overflowX: 'auto', paddingBottom: '4px' }}>
                  {[...composeAgents].sort((a, b) => a.priority - b.priority).map((agent, index, arr) => (
                    <div key={agent.id} style={{ display: 'flex', alignItems: 'center', flexShrink: 0 }}>
                      <button
                        type="button"
                        onClick={() => setExpandedAgentId(expandedAgentId === agent.id ? null : agent.id)}
                        style={{
                          display: 'flex',
                          flexDirection: 'column',
                          alignItems: 'center',
                          gap: '4px',
                          padding: '8px 14px',
                          borderRadius: '10px',
                          border: `1px solid ${expandedAgentId === agent.id ? tokens.colors.accentPrimary : 'rgba(255, 255, 255, 0.12)'}`,
                          background: agent.enabled ? 'rgba(88, 101, 242, 0.14)' : 'rgba(255, 255, 255, 0.03)',
                          color: agent.enabled ? tokens.colors.textPrimary : tokens.colors.textMuted,
                          cursor: 'pointer',
                          minWidth: '90px',
                          opacity: agent.enabled ? 1 : 0.55,
                        }}
                      >
                        <span style={{ fontSize: tokens.typography.sizeXs, fontWeight: 700, whiteSpace: 'nowrap' }}>{agent.name}</span>
                        <span style={{ fontSize: '10px', color: tokens.colors.textMuted }}>{agent.speed}</span>
                      </button>
                      {index < arr.length - 1 && (
                        <span style={{ color: tokens.colors.textMuted, padding: '0 4px', fontSize: tokens.typography.sizeMd }}>&rarr;</span>
                      )}
                    </div>
                  ))}
                  <Button variant="ghost" size="sm" onClick={addAgent} style={{ flexShrink: 0, alignSelf: 'center' }}>
                    + Add Agent
                  </Button>
                </div>

                {composeAgents.length === 0 && (
                  <div style={helperTextStyle}>No agents configured - Compose will just pass raw text through untouched. Add one to get started.</div>
                )}

                {expandedAgentId && composeAgents.find((a) => a.id === expandedAgentId) && (() => {
                  const agent = composeAgents.find((a) => a.id === expandedAgentId)!;
                  return (
                    <div id={`agent-editor-${agent.id}`} style={{ display: 'flex', flexDirection: 'column', gap: tokens.spacing.sm, padding: tokens.spacing.md, borderRadius: '10px', border: '1px solid rgba(255, 255, 255, 0.1)', background: 'rgba(255, 255, 255, 0.02)' }}>
                      <div style={{ display: 'flex', gap: tokens.spacing.sm, alignItems: 'end' }}>
                        <div style={{ display: 'flex', flexDirection: 'column', gap: '5px', flex: 1 }}>
                          <FieldLabel label="Agent name" />
                          <input style={inputBaseStyle} type="text" value={agent.name} onChange={(e: Event) => updateAgent(agent.id, { name: (e.target as HTMLInputElement).value })} placeholder="Agent name" />
                        </div>
                        <label style={{ display: 'inline-flex', alignItems: 'center', gap: '7px', color: tokens.colors.textSecondary, fontSize: tokens.typography.sizeXs }}>
                          <Switch checked={agent.enabled} onChange={(checked) => updateAgent(agent.id, { enabled: checked })} />
                          Enabled
                        </label>
                      </div>

                      <div style={{ display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) 190px auto', gap: tokens.spacing.md, alignItems: 'end' }}>
                        <div style={{ display: 'flex', flexDirection: 'column', gap: '5px' }}>
                          <div style={{ display: 'flex', justifyContent: 'space-between', color: tokens.colors.textMuted, fontSize: tokens.typography.sizeXs }}>
                            <FieldLabel label="Correction context" help="Controls how much recent refined text remains editable so later speech can clarify or correct earlier recognition." />
                            <strong style={{ color: tokens.colors.textPrimary }}>{contextStepLabel(config.compose_context_sentences)}</strong>
                          </div>
                          <input
                            type="range"
                            min={0}
                            max={CONTEXT_STEPS.length - 1}
                            step={1}
                            value={contextStepIndex(config.compose_context_sentences)}
                            onInput={(e: Event) => updateConfig('compose_context_sentences', CONTEXT_STEPS[Number((e.target as HTMLInputElement).value)].value)}
                            style={{ width: '100%' }}
                            aria-label="Correction context"
                          />
                        </div>
                        <div style={{ display: 'flex', flexDirection: 'column', gap: '5px' }}>
                          <FieldLabel label="Correction timing" help="How long Compose waits for more speech before processing the current correction window." />
                          <div style={selectWrapperStyle}>
                            <SelectField
                              value={config.compose_edit_latency}
                              onChange={(value) => updateConfig('compose_edit_latency', value)}
                              searchable={false}
                              ariaLabel="Correction timing"
                              options={[
                                { value: 'low', label: 'Quick' },
                                { value: 'medium', label: 'Balanced' },
                                { value: 'high', label: 'Patient' },
                              ]}
                            />
                          </div>
                        </div>
                        <label style={{ display: 'inline-flex', alignItems: 'center', gap: '7px', color: tokens.colors.textSecondary, fontSize: tokens.typography.sizeXs, paddingBottom: '5px' }}>
                          <Switch checked={config.compose_pause_only} onChange={(checked) => updateConfig('compose_pause_only', checked)} />
                          Long pauses only
                        </label>
                      </div>

                      <div style={{ display: 'flex', alignItems: 'center', gap: tokens.spacing.md, flexWrap: 'wrap' }}>
                        <label style={{ display: 'inline-flex', alignItems: 'center', gap: '7px', color: tokens.colors.textSecondary, fontSize: tokens.typography.sizeXs }}>
                          <Switch checked={agent.includeHistory} onChange={(checked) => updateAgent(agent.id, { includeHistory: checked })} />
                          Use recent history
                        </label>
                        <FieldLabel label="History context" help="Supplies recent saved History entries as read-only background so this agent can resolve names, terminology, and references. Private mode never supplies saved history." />
                        {agent.includeHistory && (
                          <div style={{ display: 'inline-flex', alignItems: 'center', gap: '7px' }}>
                            <NumberField value={agent.historyItems || 3} onChange={(value) => updateAgent(agent.id, { historyItems: value })} min={1} max={20} step={1} />
                            <span style={{ color: tokens.colors.textMuted, fontSize: tokens.typography.sizeXs }}>entries</span>
                          </div>
                        )}
                      </div>

                      <div style={{ display: 'flex', gap: tokens.spacing.sm, alignItems: 'end' }}>
                        <div style={{ display: 'flex', flexDirection: 'column', gap: '5px', flex: 1 }}>
                          <FieldLabel label="Profile" help="Loads a reusable prompt and fidelity starting point. Local edits remain specific to this agent until saved as a profile." />
                          <div style={selectWrapperStyle}>
                          <SelectField
                            value={agent.presetId || ''}
                            onChange={(val) => applyPresetToAgent(agent.id, val)}
                            searchable={false}
                            ariaLabel="Load preset"
                            placeholder="Load a profile..."
                            options={agentPresets.map((p) => ({
                              value: p.id,
                              label: `${p.name}${p.builtin ? ' (built-in)' : ''} — fidelity ${p.minFidelity.toFixed(2)}`,
                            }))}
                          />
                          </div>
                        </div>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => resetAgentToPreset(agent.id)}
                          disabled={!agent.presetId || (agent.prompt === agentPresets.find((p) => p.id === agent.presetId)?.prompt && agent.minFidelity === agentPresets.find((p) => p.id === agent.presetId)?.minFidelity)}
                          title="Discard edits, restore the loaded preset's original text and fidelity threshold"
                        >
                          Reset
                        </Button>
                      </div>

                      <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
                        <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: tokens.typography.sizeXs, color: tokens.colors.textMuted }}>
                          <FieldLabel label="Fidelity threshold" help="Minimum word similarity required before output is accepted. Rewrite profiles need a lower threshold than literal cleanup." />
                          <strong style={{ color: tokens.colors.textPrimary }}>{agent.minFidelity.toFixed(2)}</strong>
                        </div>
                        <input
                          type="range"
                          min={0}
                          max={1}
                          step={0.05}
                          value={agent.minFidelity}
                          onInput={(e: Event) => updateAgent(agent.id, { minFidelity: Number((e.target as HTMLInputElement).value) })}
                          style={{ width: '100%' }}
                          aria-label="Fidelity threshold"
                        />
                      </div>

                      <div style={{ display: 'flex', flexDirection: 'column', gap: '5px' }}>
                        <FieldLabel label="Prompt" help="Instructions this agent receives for every correction. Transcript content is supplied separately." />
                        <textarea
                          style={{ ...inputBaseStyle, minHeight: '120px', fontFamily: tokens.typography.fontMono, fontSize: tokens.typography.sizeXs, resize: 'vertical' }}
                          value={agent.prompt}
                          onChange={(e: Event) => updateAgent(agent.id, { prompt: (e.target as HTMLTextAreaElement).value })}
                          placeholder="What should this agent do to the text?"
                        />
                      </div>

                      {savingPresetForAgentId === agent.id ? (
                        <div style={{ display: 'flex', gap: tokens.spacing.sm, alignItems: 'center' }}>
                          <input
                            style={{ ...inputBaseStyle, flex: 1 }}
                            type="text"
                            value={presetNameDraft}
                            onChange={(e: Event) => setPresetNameDraft((e.target as HTMLInputElement).value)}
                            placeholder="Preset name"
                            autoFocus
                          />
                          <Button variant="secondary" size="sm" onClick={() => saveAgentAsPreset(agent.id, presetNameDraft)} disabled={!presetNameDraft.trim()}>
                            Save
                          </Button>
                          <Button variant="ghost" size="sm" onClick={() => { setSavingPresetForAgentId(null); setPresetNameDraft(''); }}>
                            Cancel
                          </Button>
                        </div>
                      ) : (
                        <div style={{ display: 'flex' }}>
                          <Button
                            variant="secondary"
                            size="sm"
                            onClick={() => {
                              const current = agent.presetId ? agentPresets.find((p) => p.id === agent.presetId) : null;
                              setPresetNameDraft(current && !current.builtin ? current.name : agent.name);
                              setSavingPresetForAgentId(agent.id);
                            }}
                          >
                            Save as profile…
                          </Button>
                        </div>
                      )}

                      <div style={{ display: 'flex', gap: tokens.spacing.sm, alignItems: 'center', flexWrap: 'wrap' }}>
                        <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
                          <FieldLabel label="Speed" help="Controls this agent's output budget. Faster settings are narrower; thorough settings allow more processing." />
                          <div style={selectWrapperStyle}>
                            <SelectField
                              value={agent.speed}
                              onChange={(val) => updateAgent(agent.id, { speed: val as 'low' | 'medium' | 'high' })}
                              searchable={false}
                              ariaLabel="Agent speed"
                              options={[
                                { value: 'low', label: 'Low - fast, narrow token budget' },
                                { value: 'medium', label: 'Medium' },
                                { value: 'high', label: 'High - slower, more thorough' },
                              ]}
                            />
                          </div>
                        </div>
                        <div style={{ display: 'flex', gap: '4px', marginLeft: 'auto' }}>
                          <Button variant="ghost" size="sm" onClick={() => moveAgent(agent.id, -1)} title="Move earlier in the chain">&larr;</Button>
                          <Button variant="ghost" size="sm" onClick={() => moveAgent(agent.id, 1)} title="Move later in the chain">&rarr;</Button>
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => {
                              removeAgent(agent.id);
                              setExpandedAgentId(null);
                            }}
                            style={{ color: tokens.colors.error }}
                          >
                            Remove
                          </Button>
                        </div>
                      </div>
                    </div>
                  );
                })()}
              </div>
            </ConfigField>

            </>)}
          </CollapsibleSection>
        )}

        <CollapsibleSection title="Speech recognition" isOpen={activeConfigSection === 'transcription'} onToggle={() => setActiveConfigSection(activeConfigSection === 'transcription' ? null : 'transcription')}>
          <ConfigField
            label="Recognition backend"
            description="Faster Whisper is the default high-efficiency backend and runs through CTranslate2 on CUDA or the processor. whisper.cpp remains available as the broad-hardware compatibility fallback."
          >
            <div style={selectWrapperStyle}>
              <SelectField
                value={config.local_engine}
                onChange={(value) => updateConfig('local_engine', value)}
                ariaLabel="Recognition backend"
                searchable={false}
                options={[
                  { value: 'VoxBridge Faster Whisper', label: 'VoxBridge · Faster Whisper (Default)' },
                  { value: 'VoxBridge', label: 'VoxBridge · whisper.cpp (Compatibility)' },
                ]}
              />
            </div>
          </ConfigField>

          <ConfigField label="Speech recognition model" description="Choose the local model that converts speech to text. Distil-Small is recommended for most systems.">
            <ModelSelectionPanel
              availableModels={availableModels}
              localEngine={config.local_engine}
              localModelSize={config.local_model_size}
              modelStatus={modelStatus}
              isDownloading={isDownloading}
              downloadProgress={downloadProgress}
              onChangeModel={(size) => updateConfig('local_model_size', size)}
              onShowModelGuide={() => setShowModelGuide(true)}
              onDownloadModel={downloadModel}
              onRetryModels={loadModels}
            />
          </ConfigField>

          <ConfigField
            label="Speech recognition graphics acceleration"
            description={
              isModelLoading
                ? 'Waiting for the current model load to finish before this can be changed.'
                : gpuVramCheck && !gpuVramCheck.supported
                  ? gpuVramCheck.reason ?? 'Graphics acceleration is not available on this hardware.'
                  : config.local_engine === 'VoxBridge Faster Whisper'
                    ? 'Uses CTranslate2 CUDA acceleration on supported NVIDIA systems and automatically falls back to optimized processor inference when CUDA is unavailable.'
                    : 'Uses VoxBridge Vulkan acceleration for speech recognition and falls back to the main processor when required. Usage and activity appear on Status.'
            }
          >
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: tokens.spacing.sm, width: '100%' }}>
              <IconRocket size={20} color={config.enable_gpu ? '#f1c40f' : tokens.colors.textMuted} />
              <Switch
                checked={config.enable_gpu}
                disabled={isModelLoading}
                onChange={(checked) => updateConfig('enable_gpu', checked)}
              />
            </div>
          </ConfigField>

          <ConfigField
            label="Custom vocabulary"
            description="Comma-separated names and technical terms that bias speech recognition toward the intended spelling. This fixed list is supplied with each recording."
          >
            <input
              style={inputBaseStyle}
              type="text"
              value={config.custom_vocabulary}
              onChange={(e: Event) => updateConfig('custom_vocabulary', (e.target as HTMLInputElement).value)}
              placeholder="e.g. VoxBridge, Kubernetes, Nguyen"
            />
          </ConfigField>

          <ConfigField
            label="Recording shortcut"
            description="Runs the same Start/Stop recording action shown in Compose. Press it once to begin recording and again to stop and transcribe."
          >
            <div style={{ display: 'flex', gap: tokens.spacing.sm, alignItems: 'center', justifyContent: 'center', width: '100%' }}>
              {!isSystemManagedShortcut && (
                <input
                  type="text"
                  value={config.hotkey}
                  readOnly
                  onClick={() => {}}
                  placeholder="Configure using button"
                  style={{ ...inputBaseStyle, opacity: portalVersion >= 1 ? 0.9 : 1, cursor: 'default' }}
                  title={portalVersion >= 1 ? 'Use Modify to request a recording shortcut through the system portal.' : ''}
                />
              )}
              <Button
                size="md"
                variant="configAction"
                onClick={handleConfigureHotkey}
                disabled={isApplyingHotkey}
              >
                Modify
              </Button>
            </div>
            {!isSystemManagedShortcut && portalVersion >= 1 && (
              <div style={helperTextStyle}>
                Shortcut registration uses the Wayland GlobalShortcuts portal.
                {hotkeyBindingState?.active_trigger ? ` Active shortcut: ${hotkeyBindingState.active_trigger}.` : ''}
                {hotkeyBindingState?.bound ? ' Listener is active.' : ''}
              </div>
            )}
          </ConfigField>

          <ConfigField
            label="Recording mode"
            description={
              config.hotkey_mode === 'continuous'
                ? 'Start once and keep speaking. Each pause creates recognized text; stop when the session is finished.'
                : config.hotkey_mode === 'hold'
                  ? 'Record only while the shortcut is held. Releasing it stops and transcribes.'
                  : 'The Compose button or recording shortcut starts recording; using either again stops and transcribes.'
            }
          >
            <div style={selectWrapperStyle}>
              <SelectField
                value={config.hotkey_mode}
                onChange={(val) => updateConfig('hotkey_mode', val)}
                searchable={false}
                ariaLabel="Recording mode"
                options={[
                  { value: 'sticky', label: 'Start / stop' },
                  { value: 'hold', label: 'Hold to record' },
                  { value: 'continuous', label: 'Continuous capture' },
                ]}
              />
            </div>
          </ConfigField>

          {config.hotkey_mode === 'continuous' && (
            <ConfigField label="Pause duration in milliseconds" description="How long a pause must last before speech is transcribed. Lower values respond sooner; higher values allow longer pauses within a sentence.">
              <NumberField value={config.continuous_silence_ms} onChange={(value) => updateConfig('continuous_silence_ms', value)} min={300} max={3000} step={100} />
            </ConfigField>
          )}

          <ConfigField label="End padding in milliseconds" description="Extra audio captured after recording stops, which helps preserve the end of the final word or sentence.">
            <NumberField
              value={config.post_roll_ms}
              onChange={(value) => updateConfig('post_roll_ms', value)}
              min={0}
              max={2000}
              step={50}
            />
          </ConfigField>

        </CollapsibleSection>

        <CollapsibleSection title="Audio" isOpen={activeConfigSection === 'audio'} onToggle={() => setActiveConfigSection(activeConfigSection === 'audio' ? null : 'audio')}>
          <ConfigField label="Microphone" description="Select the input device used for recording.">
            <div style={selectWrapperStyle}>
              <SelectField
                value={config.audio_device || 'default'}
                options={availableMics.map((mic) => ({ value: mic.id, label: mic.label }))}
                onChange={(nextMicId) => updateConfig('audio_device', nextMicId)}
                ariaLabel="Microphone"
              />
              <Button variant="icon" onClick={loadMics} title="Refresh devices">
                <IconRefresh size={16} />
              </Button>
            </div>
          </ConfigField>

          <ConfigField label="Microphone sensitivity" description="Adjust the input level and test the selected microphone.">
            <MicSetupPanel
              inputSensitivity={config.input_sensitivity}
              onInputSensitivityChange={(value) => updateConfig('input_sensitivity', value)}
              micTestStatus={micTestStatus}
              micVolume={micVolume}
              onStartMicTest={startMicTest}
              onStopMicTest={stopMicTest}
              onStopMicPlayback={stopMicPlayback}
            />
          </ConfigField>
        </CollapsibleSection>

        <CollapsibleSection title="Data & logs" isOpen={activeConfigSection === 'logs'} onToggle={() => setActiveConfigSection(activeConfigSection === 'logs' ? null : 'logs')}>
          <ConfigField label="Diagnostic logs" description="Open logs for troubleshooting and support.">
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', gap: tokens.spacing.sm, flexWrap: 'wrap', width: '100%' }}>
              <Button variant="secondary" size="sm" onClick={openDebugFolder}>Open logs</Button>
            </div>
          </ConfigField>

          <ConfigField label="Recording logs" description="Saves recordings as audio files for troubleshooting when history is enabled. Private mode always prevents these files from being written.">
            <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'stretch', gap: tokens.spacing.sm, width: '100%' }}>
              <Switch checked={config.enable_recording_logs} onChange={(checked) => updateConfig('enable_recording_logs', checked)} />
              {config.enable_recording_logs && (
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: tokens.spacing.sm }}>
                  <span style={{ color: tokens.colors.textSecondary, fontSize: tokens.typography.sizeXs }}>Keep recordings for</span>
                  <div style={{ display: 'flex', alignItems: 'center', gap: '7px' }}>
                    <NumberField value={config.recording_log_retention_days} onChange={(value) => updateConfig('recording_log_retention_days', value)} min={1} max={365} step={1} />
                    <span style={{ color: tokens.colors.textMuted, fontSize: tokens.typography.sizeXs }}>days</span>
                  </div>
                </div>
              )}
              <div style={{ display: 'flex', justifyContent: 'flex-end', gap: tokens.spacing.sm, width: '100%' }}>
                <Button variant="ghost" size="sm" onClick={openDebugFolder}>Open folder</Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    setRecordingLogStatus(null);
                    invoke<number>('clear_recording_logs')
                      .then((count) => setRecordingLogStatus(count ? `Removed ${count} recording${count === 1 ? '' : 's'}.` : 'No recordings to remove.'))
                      .catch((error) => setRecordingLogStatus(`Could not clear recordings: ${String(error)}`));
                  }}
                >
                  Clear recordings
                </Button>
              </div>
              {recordingLogStatus && <div style={helperTextStyle}>{recordingLogStatus}</div>}
            </div>
          </ConfigField>

        </CollapsibleSection>
        </div>
      </div>
    </div>
  );
}
