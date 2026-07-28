import { open } from '@tauri-apps/plugin-shell';
import { invoke } from '@tauri-apps/api/core';
import { IconBrandGithub, IconBug, IconCoffee, IconExternalLink, IconHeart, IconRobot, IconScale } from '@tabler/icons-preact';
import { useEffect, useState } from 'preact/hooks';
import { Button } from '../components/Button.tsx';
import { tokens } from '../design-tokens.ts';
import { tabPanelPaddedStyle, tabPanelStyle } from '../theme/ui-primitives.ts';

interface AboutPageProps {
  appVersion: string;
  onReportBug: () => void;
}

const VOXBRIDGE_REPOSITORY = 'https://github.com/tednv/VoxBridge-Compose';
const SUPPORT_URL = 'https://buymeacoffee.com/tednv';
const VOQUILL_REPOSITORY = 'https://github.com/jackbrumley/voquill';
const VOQUILL_WEBSITE = 'https://voquill.org/';
const VOQUILL_DONATE = 'https://voquill.org/donate.html';

const panelStyle = {
  border: '1px solid rgba(255,255,255,.08)',
  borderRadius: tokens.radii.panel,
  background: 'linear-gradient(145deg, rgba(255,255,255,.035), rgba(255,255,255,.012))',
  padding: '22px',
} as const;

export function AboutPage({ appVersion, onReportBug }: AboutPageProps) {
  const [hardware, setHardware] = useState<Array<[string, string]>>([]);

  useEffect(() => {
    invoke<string>('get_hardware_report')
      .then((report) => {
        const labels: Record<string, string> = {
          OS: 'Operating system',
          CPU: 'Processor',
          GPU: 'Graphics adapter',
          'System memory': 'System memory',
          'Graphics memory': 'Graphics memory',
          'Vulkan runtime available': 'Vulkan',
        };
        setHardware(
          report
            .split(/\r?\n/)
            .map((line) => line.match(/^([^:]+):\s*(.+)$/))
            .filter((match): match is RegExpMatchArray => !!match && !!labels[match[1]])
            .map((match) => [labels[match[1]], match[2]]),
        );
      })
      .catch(() => setHardware([]));
  }, []);

  return (
    <div style={tabPanelStyle}>
      <div style={{ ...tabPanelPaddedStyle, maxWidth: '980px', gap: '14px' }}>
        <section style={{ ...panelStyle, borderColor: 'rgba(255,138,0,.22)', background: 'linear-gradient(145deg, rgba(255,138,0,.09), rgba(255,255,255,.015))' }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: '9px', color: tokens.colors.accentSoft, marginBottom: '12px' }}>
            <IconHeart size={18} />
            <span style={{ fontSize: '12px', fontWeight: 700, letterSpacing: '.06em', textTransform: 'uppercase' }}>With gratitude to FOSS Voquill</span>
          </div>
          <h1 style={{ margin: '0 0 12px', fontSize: '25px', lineHeight: 1.2, fontWeight: 700 }}>
            Built on an open-source foundation
          </h1>
          <div style={{ display: 'grid', gap: '10px', maxWidth: '820px', color: tokens.colors.textSecondary, fontSize: '13px', lineHeight: 1.65 }}>
            <p style={{ margin: 0 }}>
              VoxBridge Compose originated from work based on <strong style={{ color: tokens.colors.textPrimary }}>FOSS Voquill</strong>, created by Jack Brumley with contributions from its open-source community. Voquill’s commitment to free, private, local dictation provided both an important technical foundation and the inspiration to keep this work open.
            </p>
            <p style={{ margin: 0 }}>
              Portions of the application may retain or derive from Voquill’s GNU Affero General Public License version 3 code. The original project, its authors, and its contributors deserve generous credit for making that work available for others to study, adapt, and extend.
            </p>
            <p style={{ margin: 0, color: tokens.colors.textMuted }}>
              VoxBridge Compose is independently maintained and is not affiliated with or endorsed by the original Voquill maintainers. Original contributions remain copyright their respective authors.
            </p>
          </div>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: '9px', marginTop: '16px' }}>
            <Button variant="secondary" size="sm" onClick={() => void open(VOQUILL_REPOSITORY)}>
              <IconBrandGithub size={15} /> Visit the original FOSS Voquill project <IconExternalLink size={13} />
            </Button>
            <Button variant="secondary" size="sm" onClick={() => void open(VOQUILL_WEBSITE)}>
              Voquill website <IconExternalLink size={13} />
            </Button>
            <Button variant="secondary" size="sm" onClick={() => void open(VOQUILL_DONATE)}>
              <IconCoffee size={15} /> Support FOSS Voquill <IconExternalLink size={13} />
            </Button>
          </div>
        </section>

        <section style={{ ...panelStyle, display: 'grid', gridTemplateColumns: 'minmax(0, 1.4fr) minmax(260px, .8fr)', gap: '24px', alignItems: 'start' }}>
          <div>
            <div style={{ display: 'flex', alignItems: 'baseline', gap: '10px', flexWrap: 'wrap' }}>
              <h2 style={{ margin: 0, fontSize: '21px' }}><span style={{ color: tokens.colors.accentPrimary }}>Vox</span>Bridge Compose</h2>
              <span style={{ color: tokens.colors.textMuted, fontSize: '12px', fontFamily: tokens.typography.fontMono }}>v{appVersion || '0.1.0'}</span>
            </div>
            <p style={{ margin: '10px 0 0', color: tokens.colors.textSecondary, fontSize: '13px', lineHeight: 1.6 }}>
              A local-first workspace for accelerated speech recognition and ordered text refinement. Recognition and refinement can remain on your hardware without requiring an account or hosted service.
            </p>
          </div>
          <div style={{ display: 'grid', gap: '9px' }}>
            <Button variant="primary" onClick={() => void open(SUPPORT_URL)}>
              <IconRobot size={16} /> Buy me some LLM tokens <IconExternalLink size={13} />
            </Button>
            <Button variant="secondary" onClick={() => void open(VOXBRIDGE_REPOSITORY)}>
              <IconBrandGithub size={16} /> Source repository <IconExternalLink size={13} />
            </Button>
          </div>
        </section>

        <section style={panelStyle}>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: '12px', marginBottom: '14px' }}>
            <h2 style={{ margin: 0, fontSize: '16px' }}>This system</h2>
            <Button variant="ghost" size="sm" onClick={onReportBug}>
              <IconBug size={15} /> Report a bug
            </Button>
          </div>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, minmax(0, 1fr))', borderTop: '1px solid rgba(255,255,255,.08)', borderLeft: '1px solid rgba(255,255,255,.08)' }}>
            {(hardware.length ? hardware : [['Hardware', 'Loading…']]).map(([label, value]) => (
              <div key={label} style={{ minHeight: '66px', padding: '12px 14px', borderRight: '1px solid rgba(255,255,255,.08)', borderBottom: '1px solid rgba(255,255,255,.08)' }}>
                <div style={{ color: tokens.colors.textMuted, fontSize: '10px', letterSpacing: '.06em', textTransform: 'uppercase' }}>{label}</div>
                <div style={{ marginTop: '6px', color: tokens.colors.textPrimary, fontSize: '12px', lineHeight: 1.45 }}>{value}</div>
              </div>
            ))}
          </div>
        </section>

        <section style={{ ...panelStyle, display: 'flex', gap: '16px', alignItems: 'flex-start' }}>
          <span style={{ display: 'grid', placeItems: 'center', width: '38px', height: '38px', flex: '0 0 auto', borderRadius: '9px', color: tokens.colors.accentSoft, background: 'rgba(255,138,0,.1)' }}>
            <IconScale size={20} />
          </span>
          <div>
            <h2 style={{ margin: '1px 0 7px', fontSize: '16px' }}>GNU Affero General Public License version 3</h2>
            <p style={{ margin: 0, color: tokens.colors.textSecondary, fontSize: '13px', lineHeight: 1.6 }}>
              VoxBridge Compose is free software distributed under the AGPL-3.0-only license. You may use, inspect, modify, and share it under the license terms. Network-accessible modified versions must also make their corresponding source available as required by the AGPL.
              The software is provided without warranty; see the repository&apos;s LICENSE file for the complete terms and corresponding source.
            </p>
          </div>
        </section>
      </div>
    </div>
  );
}
