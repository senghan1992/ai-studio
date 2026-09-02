import React, { useState } from 'react';

/**
 * Office's table grid: hover to size, click to insert.
 *
 * Deliberately the same interaction — people size a table by sweeping this grid
 * and read the count in the caption, and any other control here would be a
 * relearn for no gain.
 */
const MAX_COLS = 10;
const MAX_ROWS = 8;

export default function TablePicker({ onPick, onClose }) {
  const [hover, setHover] = useState({ cols: 0, rows: 0 });

  return (
    <div className="tablepicker" role="dialog" aria-label="표 삽입">
      <div
        className="tablepicker__grid"
        onMouseLeave={() => setHover({ cols: 0, rows: 0 })}
        role="grid"
      >
        {Array.from({ length: MAX_ROWS }, (_, r) => (
          <div className="tablepicker__row" key={r} role="row">
            {Array.from({ length: MAX_COLS }, (_, c) => (
              <button
                key={c}
                role="gridcell"
                aria-label={`${c + 1}열 ${r + 1}행`}
                className={`tablepicker__cell${
                  c < hover.cols && r < hover.rows ? ' is-on' : ''
                }`}
                onMouseEnter={() => setHover({ cols: c + 1, rows: r + 1 })}
                onFocus={() => setHover({ cols: c + 1, rows: r + 1 })}
                onClick={() => {
                  onPick(c + 1, r + 1);
                  onClose?.();
                }}
              />
            ))}
          </div>
        ))}
      </div>
      <div className="tablepicker__caption">
        {hover.cols > 0 ? `${hover.cols} × ${hover.rows} 표` : '표 크기를 선택하세요'}
      </div>
    </div>
  );
}
