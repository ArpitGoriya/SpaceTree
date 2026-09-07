import { useCallback, useState } from 'react';

import { api } from './api';
import type { HeaderDto, ScanProgressDto } from './api';
import Launcher from './screens/Launcher';
import Scanning from './screens/Scanning';
import Results from './screens/Results';

type Screen = { name: 'launcher' } | { name: 'scanning'; path: string } | { name: 'results'; header: HeaderDto };

export default function App() {
  const [screen, setScreen] = useState<Screen>({ name: 'launcher' });
  const [error, setError] = useState<string | null>(null);
  const [lastProgress, setLastProgress] = useState<ScanProgressDto | null>(null);

  const startScan = useCallback(async (path: string) => {
    setError(null);
    setLastProgress(null);
    setScreen({ name: 'scanning', path });
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
        <Scanning path={screen.path} onCancel={cancelScan} onProgress={setLastProgress} progress={lastProgress} />
      )}
      {screen.name === 'results' && <Results initialHeader={screen.header} onClose={backToLauncher} />}
    </div>
  );
}
