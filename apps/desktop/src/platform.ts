// Platform facts the interface needs in order to phrase itself honestly.
//
// Pure: no DOM, no IPC, so the Windows branch is testable from macOS. The platform is
// read from the resolved home directory rather than the user agent, because the home path
// is the platform's own answer to "where the user's files live".

/** True for a Windows drive path (`C:\…`, `c:/…`) or a UNC share (`\\host\share`). */
export function isWindowsPath(path: string): boolean {
  return /^[A-Za-z]:[\\/]/.test(path) || path.startsWith('\\\\')
}

/**
 * The example shown in the manual-path box.
 *
 * Windows does not expand `~`, so offering `~/Documents` there promises something that
 * will not work. The hint names `%USERPROFILE%` instead, which Windows does expand, and
 * `C:\temp` for the plain drive-root form. Unix has both `/tmp` and `~`, so it keeps them.
 */
export function pathPlaceholder(home: string): string {
  if (isWindowsPath(home)) {
    return '…or type a path, e.g. C:\\temp or %USERPROFILE%\\Documents'
  }
  return '…or type a path, e.g. /tmp or ~/Documents'
}
