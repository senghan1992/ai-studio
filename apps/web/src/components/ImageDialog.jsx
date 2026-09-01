import React, { useCallback, useEffect, useRef, useState } from 'react';
import { api, assetUrl } from '../api.js';
import { Dialog, Field } from './ui.jsx';

const MAX_BYTES = 12 * 1024 * 1024;

/**
 * Insert an image: upload a file into the project's `assets/` folder, pick one
 * that is already there, or point at a URL.
 *
 * Uploading writes the file next to the document, so the markdown can keep a plain
 * relative path and the whole folder stays copyable.
 */
export default function ImageDialog({ folder, initial, onCancel, onConfirm, notify }) {
  const [src, setSrc] = useState(initial?.src ?? '');
  const [alt, setAlt] = useState(initial?.alt ?? '');
  const [assets, setAssets] = useState([]);
  const [busy, setBusy] = useState(false);
  const [dropping, setDropping] = useState(false);
  const inputRef = useRef(null);

  useEffect(() => {
    api
      .listAssets(folder)
      .then((data) => setAssets(data.assets ?? []))
      .catch(() => setAssets([]));
  }, [folder]);

  const upload = useCallback(
    async (file) => {
      if (!file) return;
      if (!file.type.startsWith('image/')) {
        notify?.('이미지 파일만 넣을 수 있습니다');
        return;
      }
      if (file.size > MAX_BYTES) {
        notify?.(`이미지가 너무 큽니다 (최대 ${Math.round(MAX_BYTES / 1024 / 1024)}MB)`);
        return;
      }
      setBusy(true);
      try {
        const dataUrl = await new Promise((resolve, reject) => {
          const reader = new FileReader();
          reader.onload = () => resolve(reader.result);
          reader.onerror = () => reject(new Error('파일을 읽을 수 없습니다'));
          reader.readAsDataURL(file);
        });
        const saved = await api.uploadAsset(folder, file.name, dataUrl);
        setAssets((list) => [...list.filter((a) => a.path !== saved.path), saved].sort((a, b) => a.path.localeCompare(b.path)));
        setSrc(`../${saved.path}`);
        if (!alt) setAlt(file.name.replace(/\.[^.]+$/, ''));
        notify?.(`${saved.path} 저장됨`);
      } catch (e) {
        notify?.(e.message);
      } finally {
        setBusy(false);
      }
    },
    [folder, alt, notify]
  );

  const previewUrl = src && !/^(https?:|data:)/i.test(src) ? assetUrl(folder, src) : src;

  return (
    <Dialog
      title={initial ? '이미지 편집' : '이미지 삽입'}
      confirmLabel={initial ? '적용' : '삽입'}
      busy={busy}
      onCancel={onCancel}
      onConfirm={() => src.trim() && onConfirm({ src: src.trim(), alt: alt.trim() })}
    >
      <div
        className={`imgdrop${dropping ? ' is-over' : ''}`}
        onDragOver={(e) => {
          e.preventDefault();
          setDropping(true);
        }}
        onDragLeave={() => setDropping(false)}
        onDrop={(e) => {
          e.preventDefault();
          setDropping(false);
          upload(e.dataTransfer?.files?.[0]);
        }}
        onClick={() => inputRef.current?.click()}
        role="button"
        tabIndex={0}
      >
        <input
          ref={inputRef}
          type="file"
          accept="image/png,image/jpeg,image/gif,image/webp,image/svg+xml"
          hidden
          onChange={(e) => upload(e.target.files?.[0])}
        />
        {busy ? '올리는 중…' : '이미지를 여기로 끌어오거나 눌러서 선택하세요'}
        <span className="imgdrop__hint">
          프로젝트의 <code>assets/</code> 폴더에 저장되고, 마크다운에는 상대 경로가 들어갑니다.
        </span>
      </div>

      {assets.length > 0 && (
        <Field label="이미 올린 이미지">
          <div className="imgpicker">
            {assets.map((asset) => (
              <button
                key={asset.path}
                type="button"
                className={`imgpicker__item${src.endsWith(asset.path) ? ' is-on' : ''}`}
                title={`${asset.path} · ${Math.round(asset.bytes / 1024)}KB`}
                onClick={() => setSrc(`../${asset.path}`)}
              >
                <img src={assetUrl(folder, asset.path)} alt="" />
              </button>
            ))}
          </div>
        </Field>
      )}

      <Field label="경로 또는 URL">
        <input value={src} onChange={(e) => setSrc(e.target.value)} placeholder="../assets/chart.png" />
      </Field>

      <Field label="대체 텍스트 (AI가 읽는 설명)">
        <input value={alt} onChange={(e) => setAlt(e.target.value)} placeholder="분기별 매출 추이 그래프" />
      </Field>

      {previewUrl && (
        <div className="imgpreview">
          <img src={previewUrl} alt={alt || '미리보기'} />
        </div>
      )}
    </Dialog>
  );
}
