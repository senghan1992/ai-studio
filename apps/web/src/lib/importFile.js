/**
 * Opening an existing Office file.
 *
 * The desktop app reads the file natively — a 40MB deck never becomes base64 —
 * while a browser has to hand the bytes over as a data URL. Both end at the same
 * `ai-core` method, so what lands on disk is identical either way.
 *
 * The conversion is one-way by design: a `.pptx` becomes a `.aideck` folder and
 * the original file is left untouched. Getting a `.pptx` back is what the export
 * path is for.
 */
import { api, isDesktop } from '../api.js';

export const OFFICE_EXTENSIONS = ['.pptx', '.docx', '.xlsx'];

/** The file input's `accept`, including the variants Office also writes. */
export const OFFICE_ACCEPT = [
  '.pptx',
  '.pptm',
  '.potx',
  '.docx',
  '.docm',
  '.dotx',
  '.xlsx',
  '.xlsm',
  '.xltx',
].join(',');

/**
 * Show a native file picker and import what the user chose.
 * Returns `null` when the dialog was dismissed — not an error.
 */
export async function pickAndImport() {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke('open_office_file');
}

/** Import a `File` from a browser input or a drop. */
export async function importFile(file) {
  const data = await readAsBase64(file);
  return api.importFile(file.name, data);
}

/** True for a filename this can open. */
export function isOfficeFile(name) {
  const lower = String(name ?? '').toLowerCase();
  return /\.(pptx|pptm|potx|docx|docm|dotx|xlsx|xlsm|xltx)$/.test(lower);
}

/** A helpful message for a file we cannot open. */
export function rejectionFor(name) {
  const lower = String(name ?? '').toLowerCase();
  if (/\.(ppt|doc|xls)$/.test(lower)) {
    const modern = `${lower.slice(lower.lastIndexOf('.'))}x`;
    return `${name}은 2007년 이전 형식입니다. Office에서 ${modern}로 저장한 뒤 열어 주세요.`;
  }
  if (/\.(pdf|key|numbers|pages|odt|ods|odp)$/.test(lower)) {
    return `${name}은 아직 열 수 없습니다. pptx · docx · xlsx만 지원합니다.`;
  }
  return `${name}은 Office 문서가 아닙니다. pptx · docx · xlsx를 선택해 주세요.`;
}

/** Read a file as bare base64, without the `data:` prefix. */
function readAsBase64(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error(`${file.name}을 읽을 수 없습니다`));
    reader.onload = () => {
      const result = String(reader.result ?? '');
      const comma = result.indexOf(',');
      resolve(comma >= 0 ? result.slice(comma + 1) : result);
    };
    reader.readAsDataURL(file);
  });
}

/** True when a native picker is available. */
export const hasNativePicker = () => isDesktop();
