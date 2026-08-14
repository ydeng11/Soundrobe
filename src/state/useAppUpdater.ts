import { useCallback, useEffect, useRef, useState } from "react";
import type {
  AppUpdateInfo,
  AppUpdateProgress,
} from "../shared/desktop-api";

function errorMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

export function useAppUpdater(busy: boolean) {
  const startupCheckStarted = useRef(false);
  const [supported, setSupported] = useState(false);
  const [update, setUpdate] = useState<AppUpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  const [checkMessage, setCheckMessage] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<AppUpdateProgress | null>(null);
  const [installError, setInstallError] = useState<string | null>(null);

  const check = useCallback(async (manual: boolean) => {
    setChecking(true);
    if (manual) setCheckMessage(null);
    try {
      const available = await window.api.checkForUpdate();
      setUpdate(available);
      if (manual) {
        setCheckMessage(
          available
            ? `Soundrobe ${available.availableVersion} is available.`
            : "Soundrobe is up to date.",
        );
      }
    } catch (reason) {
      if (manual) {
        setCheckMessage(`Could not check for updates: ${errorMessage(reason)}`);
      }
    } finally {
      setChecking(false);
    }
  }, []);

  useEffect(() => {
    if (startupCheckStarted.current) return;
    startupCheckStarted.current = true;
    void window.api
      .appInfo()
      .then((info) => {
        if (info.dev) return;
        setSupported(true);
        return check(false);
      })
      .catch(() => {});
  }, [check]);

  const checkManually = useCallback(async () => {
    if (!supported) {
      setCheckMessage("Updates are available in packaged production builds.");
      return;
    }
    await check(true);
  }, [check, supported]);

  const install = useCallback(async () => {
    if (busy) {
      setInstallError(
        "Finish the current disk operation before installing the update.",
      );
      return;
    }
    setInstalling(true);
    setInstallError(null);
    setProgress({ phase: "downloading", downloaded: 0, total: null });
    try {
      await window.api.installUpdate(setProgress);
    } catch (reason) {
      setInstallError(errorMessage(reason));
      setInstalling(false);
    }
  }, [busy]);

  const dismiss = useCallback(() => {
    if (!installing) {
      setUpdate(null);
      setInstallError(null);
      setProgress(null);
    }
  }, [installing]);

  return {
    update,
    supported,
    checking,
    checkMessage,
    installing,
    progress,
    installError,
    checkManually,
    install,
    dismiss,
  };
}
