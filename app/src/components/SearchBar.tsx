import { forwardRef } from 'react';

const SearchBar = forwardRef<HTMLInputElement, { value: string; onChange: (v: string) => void }>(
  ({ value, onChange }, ref) => {
    return (
      <div style={{ position: 'relative', flex: '0 1 280px' }}>
        <input
          ref={ref}
          type="search"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder="Search  (/  to focus, *.ext supported)"
          style={{ width: '100%' }}
        />
        {value && (
          <button
            onClick={() => onChange('')}
            style={{
              position: 'absolute',
              right: 4,
              top: '50%',
              transform: 'translateY(-50%)',
              padding: '2px 6px',
              fontSize: 'var(--text-label)',
            }}
          >
            ×
          </button>
        )}
      </div>
    );
  },
);
SearchBar.displayName = 'SearchBar';

export default SearchBar;
