import { invoke } from '@tauri-apps/api/core';

import type { AppInfo } from '../bindings/AppInfo';
import type { CreateInstancePlanDto } from '../bindings/CreateInstancePlanDto';
import type { CreateRecoveryDto } from '../bindings/CreateRecoveryDto';
import type { InstallEventDto } from '../bindings/InstallEventDto';
import type { InstallRequest } from '../bindings/InstallRequest';
import type { InstallResultDto } from '../bindings/InstallResultDto';
import type { CreateInstanceRequest } from '../bindings/CreateInstanceRequest';
import type { IpcError } from '../bindings/IpcError';
import type { InstanceSnapshotDto } from '../bindings/InstanceSnapshotDto';
import type { InstanceSummaryDto } from '../bindings/InstanceSummaryDto';
import type { InstallPlanDto } from '../bindings/InstallPlanDto';
import type { LinkInstanceRequest } from '../bindings/LinkInstanceRequest';
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
  /** جذر النسخة، لا مجلد mods مباشرة. */
  instanceRoot: string;
  action: 'disable' | 'enable';
  /** أسماء ملفات داخل mods، يُحلّها Rust تحت <instanceRoot>/mods. المسارات المطلقة تُرفض هنا. */
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
      instanceRoot: req.instanceRoot,
      action: req.action,
      files: req.files,
    },
  });
}

/** إعادة تسمية فقط؛ الخطة تُعاد بناؤها في Rust فلا يُوثق ما يصل من الواجهة. */
export function executeToggle(req: PreviewToggleRequest): Promise<ToggleResultDto[]> {
  return invoke<ToggleResultDto[]>('execute_toggle', {
    req: {
      instanceRoot: req.instanceRoot,
      action: req.action,
      files: req.files,
    },
  });
}

// --- Instances (F-101..F-104) -------------------------------------------
//
// Both create commands take the same intent. Rust rebuilds the plan, so a
// folder taken between preview and confirm fails there rather than here.

export function listInstances(): Promise<InstanceSummaryDto[]> {
  return invoke<InstanceSummaryDto[]>('list_instances');
}

export function planCreateInstance(req: CreateInstanceRequest): Promise<CreateInstancePlanDto> {
  return invoke<CreateInstancePlanDto>('plan_create_instance', { req });
}

export function executeCreateInstance(req: CreateInstanceRequest): Promise<InstanceSummaryDto> {
  return invoke<InstanceSummaryDto>('execute_create_instance', { req });
}

export function linkInstance(req: LinkInstanceRequest): Promise<InstanceSummaryDto> {
  return invoke<InstanceSummaryDto>('link_instance', { req });
}

/** Crashed instance creates awaiting a decision (F-146). */
export function listPendingCreates(): Promise<CreateRecoveryDto[]> {
  return invoke<CreateRecoveryDto[]>('list_pending_creates');
}

/** `operationId` only: core derives the folder from the record, never from the webview. */
export function finishCreate(operationId: string): Promise<InstanceSummaryDto> {
  return invoke<InstanceSummaryDto>('finish_create', { operationId });
}

/**
 * F-063. Runs the install plan and streams progress.
 *
 * The channel is the progress; the returned promise resolves when every item has
 * settled. onEvent must be registered before invoking.
 */
export function executeInstall(
  req: InstallRequest,
  onEvent: (e: InstallEventDto) => void,
): Promise<InstallResultDto> {
  return invoke<InstallResultDto>('execute_install', {
    req,
    onEvent,
  });
}

/** Opens a registered instance folder. The path is checked in Rust against the
 * registry; the webview cannot name an arbitrary folder. */
export function openInstanceFolder(root: string): Promise<void> {
  return invoke<void>('open_instance_folder', { root });
}

/** Removes the standard folders that are still empty and closes the record. */
export function abandonCreate(operationId: string): Promise<number> {
  return invoke<number>('abandon_create', { operationId });
}

/** Turn any error `invoke` rejects with into an IpcError (Tauri rejects with the serialized value itself). */
export function toIpcError(e: unknown): IpcError {
  if (typeof e === 'object' && e !== null && 'code' in e && 'message' in e) {
    return e as IpcError;
  }
  return { code: 'internal', message: String(e) };
}
