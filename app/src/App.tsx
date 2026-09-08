import { useCallback, useState } from 'react';

import { api } from './api';
import type { HeaderDto, ScanProgressDto, VolumeDto } from './api';
import Launcher from './screens/Launcher';
import Scanning from './screens/Scanning';
import Results from './screens/Results';
import Settings from './screens/Settings';

type Screen =
  | { name: 'launcher' }
  | { name: 'scanning'; path: string; volume: VolumeDto | null }
  | { name: 'results'; header: HeaderDto };

export default function App() {
  const [screen, setScreen] = useState<Screen>({ name: 'launcher' });
  const [error, setError] = useState<string | null>(null);
  const [lastProgress, setLastProgress] = useState<ScanProgressDto | null>(null);
  // Settings overlays whatever screen is showing rather than replacing
  // it, so returning from it doesn't discard a scan that took minutes.
  const [showSettings, setShowSettings] = useState(false);
  const [settingsRevision, setSettingsRevision] = useState(0);

  // `volume` is whatever the launcher already knows about the drive, so
  // the scanning screen can show progress against its used bytes instead
  // of a number with nothing to compare it to.
  const startScan = useCallback(async (path: string, volume: VolumeDto | null = null) => {
    setError(null);
    setLastProgress(null);
    setScreen({ name: 'scanning', path, volume });
    try {
      const header = await api.startScan(path);
      setScreen({ name: 'results', header });
    } catch (e) {
      setError(String(e));
      setScreen({ name: 'launcher' });
    }
  }, []);

  const cancelScan = useCallback(async () => {
    await api.cancelScan();
  }, []);

  const backToLauncher = useCallback(() => {
    setScreen({ name: 'launcher' });
  }, []);

  return (
    <div
      style={{
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        position: 'relative',
      }}
    >
      {error && (
        <div
          style={{
            padding: 'var(--space-2) var(--space-4)',
            background: 'color-mix(in srgb, var(--danger) 18%, var(--bg))',
            borderBottom: '1px solid var(--border)',
            color: 'var(--text)',
            fontSize: 'var(--text-secondary)',
          }}
        >
          {error}
        </div>
      )}
      {screen.name === 'launcher' && <Launcher onScan={startScan} />}
      {screen.name === 'scanning' && (
        <Scanning
          path={screen.path}
          volume={screen.volume}
          onCancel={cancelScan}
          onProgress={setLastProgress}
          progress={lastProgress}
        />
      )}
      {screen.name === 'results' && (
        <Results
          initialHeader={screen.header}
          onClose={backToLauncher}
          onOpenSettings={() => setShowSettings(true)}
          settingsRevision={settingsRevision}
        />
      )}

      {showSettings && (
        <div
          style={{
            position: 'absolute',
            inset: 0,
            background: 'var(--bg)',
            display: 'flex',
            flexDirection: 'column',
            zIndex: 30,
          }}
        >
          <Settings
            onClose={() => {
              setShowSettings(false);
              setSettingsRevision((r) => r + 1);
            }}
          />
        </div>
      )}
    </div>
  );
}
