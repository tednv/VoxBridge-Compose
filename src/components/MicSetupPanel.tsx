import { Button } from './Button.tsx';
import { SliderField } from './SliderField.tsx';
import { tokens } from '../design-tokens.ts';

interface MicSetupPanelProps {
  inputSensitivity: number;
  onInputSensitivityChange: (value: number) => void;
  micTestStatus: 'idle' | 'recording' | 'playing' | 'processing';
  micVolume: number;
  onStartMicTest: () => void;
  onStopMicTest: () => void;
  onStopMicPlayback: () => void;
  compact?: boolean;
  actionButtonSize?: 'sm' | 'md' | 'lg';
}

export function MicSetupPanel({
  inputSensitivity,
  onInputSensitivityChange,
  micTestStatus,
  micVolume,
  onStartMicTest,
  onStopMicTest,
  onStopMicPlayback,
  compact = false,
  actionButtonSize = 'md',
}: MicSetupPanelProps) {
  const showVolumeMeter = micTestStatus === 'recording';
  const showPlaybackText = micTestStatus === 'playing';

  return (
    <div style={{ width: '100%' }}>
      <SliderField
        value={inputSensitivity}
        min={0.1}
        max={2.0}
        step={0.05}
        onChange={onInputSensitivityChange}
        ariaLabel="Microphone sensitivity"
        style={{ margin: `${tokens.spacing.sm} 0` }}
      />

      <div
        style={{
          marginTop: compact ? tokens.spacing.sm : tokens.spacing.md,
          display: 'flex',
          flexDirection: 'column',
          gap: tokens.spacing.sm,
          alignItems: 'center',
        }}
      >
        <div style={{ width: '100%', minHeight: '18px', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          {showVolumeMeter && (
            <div style={{ width: '100%', height: '4px', background: tokens.colors.bgTertiary, borderRadius: '2px', overflow: 'hidden' }}>
              <div
                style={{
                  width: `${Math.min(micVolume * 100, 100)}%`,
                  height: '100%',
                  background: micVolume > 0.9 ? '#e74c3c' : micVolume > 0.7 ? '#f1c40f' : tokens.colors.success,
                  transition: 'width 0.1s ease-out',
                  boxShadow: micVolume > 0.9 ? '0 0 5px #e74c3c' : 'none',
                }}
              ></div>
            </div>
          )}

          {showPlaybackText && (
            <span style={{ fontSize: tokens.typography.sizeXs, color: tokens.colors.textSecondary }}>
              Playing back recording
            </span>
          )}
        </div>

        <Button
          disabled={micTestStatus === 'processing'}
          variant="secondary"
          size={actionButtonSize}
          onClick={() => {
            if (micTestStatus === 'idle') {
              onStartMicTest();
            } else if (micTestStatus === 'recording') {
              onStopMicTest();
            } else if (micTestStatus === 'playing') {
              onStopMicPlayback();
            }
          }}
        >
          {micTestStatus === 'idle'
            ? 'Test microphone'
            : micTestStatus === 'recording'
              ? 'Stop and play'
              : micTestStatus === 'playing'
                ? 'Stop playback'
                : 'Processing…'}
        </Button>
      </div>
    </div>
  );
}
