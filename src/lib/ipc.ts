import { invoke } from '@tauri-apps/api/core';

import type { AppInfo } from '../bindings/AppInfo';
import type { IpcError } from '../bindings/IpcError';
import type { InstanceSnapshotDto } from '../bindings/InstanceSnapshotDto';
import type { InstallPlanDto } from '../bindings/InstallPlanDto';
import type { SearchPageDto } from '../bindings/SearchPageDto';
import type { TogglePlanDto } from '../bindings/TogglePlanDto';
import type { ToggleResultDto } from '../bindings/ToggleResultDto';

// الواجهة تصل إلى النواة عبر هذا الملف فقط.

export interface SearchQuery {
  query: string;
  gameVersion?: string;
  loader?: string;
  projectType?: string;
  limit?: number;
  offset?: number;
}

export interface PreviewToggleRequest {
  /** مجلد mods. */
  modsDir: string;
  action: 'disable' | 'enable';
  /** أسماء ملفات داخل modsDir (لا مسارات مطلقة). */
  files: string[];
}

export function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>('app_info');
}

export function searchProjects(query: SearchQuery): Promise<SearchPageDto> {
  return invoke<SearchPageDto>('search_projects', {
    query: {
      query: query.query,
      gameVersion: query.gameVersion ?? null,
      loader: query.loader ?? null,
      projectType: query.projectType ?? 'mod',
      limit: query.limit ?? 20,
      offset: query.offset ?? 0,
    },
  });
}

/** معاينة فقط: لا تكتب ولا تنزّل. */
export function buildInstallPlan(
  slugs: string[],
  gameVersion: string,
  loader: string,
  instanceRoot?: string,
): Promise<InstallPlanDto> {
  return invoke<InstallPlanDto>('build_install_plan', {
    slugs,
    gameVersion,
    loader,
    instanceRoot: instanceRoot ?? null,
  });
}

export function scanInstance(instanceRoot: string): Promise<InstanceSnapshotDto> {
  return invoke<InstanceSnapshotDto>('scan_instance', { instanceRoot });
}

export function previewToggle(req: PreviewToggleRequest): Promise<TogglePlanDto> {
  return invoke<TogglePlanDto>('preview_toggle', {
    req: {
      modsDir: req.modsDir,
      action: req.action,
      files: req.files,
    },
  });
}

/** إعادة تسمية فقط؛ الخطة تُعاد بناؤها في Rust فلا يُوثق ما يصل من الواجهة. */
export function executeToggle(req: PreviewToggleRequest): Promise<ToggleResultDto[]> {
  return invoke<ToggleResultDto[]>('execute_toggle', {
    req: {
      modsDir: req.modsDir,
      action: req.action,
      files: req.files,
    },
  });
}

/** يحوّل أي خطأ يرفضه invoke إلى IpcError (Tauri يرفض بالقيمة المُسلسَلة نفسها). */
export function toIpcError(e: unknown): IpcError {
  if (typeof e === 'object' && e !== null && 'code' in e && 'message' in e) {
    return e as IpcError;
  }
  return { code: 'internal', message: String(e) };
}