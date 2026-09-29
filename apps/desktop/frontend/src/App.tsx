/**
 * The application root.
 *
 * Loads the settings, wires the backend event channels, applies the theme, and
 * renders the active workspace inside the shell.
 */

import { useEffect } from "react";
import { Shell } from "./components/Shell";
import { useStore, wireEvents } from "./lib/store";
import { OverviewPage } from "./pages/Overview";
import { DynamicsPage } from "./pages/Dynamics";
import { TelemetryPage } from "./pages/LiveTelemetry";
import { FlightAnalysisPage } from "./pages/FlightAnalysis";
import { ProjectsPage } from "./pages/Projects";
import { SettingsPage } from "./pages/Settings";

export function App() {
  const section = useStore((s) => s.section);
  const loadSettings = useStore((s) => s.loadSettings);
  const applyAppearance = useStore((s) => s.applyAppearance);
  const refreshProject = useStore((s) => s.refreshProject);
  const loadModels = useStore((s) => s.loadModels);
  const setSystemPrefersDark = useStore.setState;

  useEffect(() => {
    void loadSettings();
    void refreshProject();
    void loadModels();
  }, [loadSettings, refreshProject, loadModels]);

  useEffect(() => {
    const unsubscribe = wireEvents();
    return unsubscribe;
  }, []);

  // Follow the operating system preference while the theme is set to System, so
  // the interface does not need a restart when the desktop switches at dusk.
  useEffect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const query = window.matchMedia("(prefers-color-scheme: dark)");
    const handler = (event: MediaQueryListEvent) => {
      setSystemPrefersDark({ systemPrefersDark: event.matches });
      applyAppearance();
    };
    query.addEventListener("change", handler);
    return () => query.removeEventListener("change", handler);
  }, [applyAppearance, setSystemPrefersDark]);

  return (
    <Shell>
      {section === "overview" && <OverviewPage />}
      {section === "dynamics" && <DynamicsPage />}
      {section === "telemetry" && <TelemetryPage />}
      {section === "analysis" && <FlightAnalysisPage />}
      {section === "projects" && <ProjectsPage />}
      {section === "settings" && <SettingsPage />}
    </Shell>
  );
}

export default App;
