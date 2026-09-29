/**
 * The Projects screen: create, open, and remove projects.
 *
 * A project is a directory with explicit metadata, so this screen is mostly a
 * directory picker plus a readable summary of what a project contains.
 */

import { useEffect, useState } from "react";
import {
  Badge,
  Button,
  Checkbox,
  DataTable,
  EmptyState,
  Field,
  Hint,
  Notice,
  Panel,
  TextInput,
} from "../components/ui";
import { IconOpen, IconPlus, IconProjects, IconTrash } from "../components/Icons";
import { useStore } from "../lib/store";
import { pickDirectory } from "../lib/api";
import { formatRelative, formatTimestamp } from "../lib/format";

export function ProjectsPage() {
  const project = useStore((s) => s.project);
  const recent = useStore((s) => s.recentProjects);
  const discovered = useStore((s) => s.discovered);
  const busy = useStore((s) => s.busy);
  const createProject = useStore((s) => s.createProject);
  const openProject = useStore((s) => s.openProject);
  const saveProject = useStore((s) => s.saveProject);
  const discoverProjects = useStore((s) => s.discoverProjects);
  const updateProject = useStore((s) => s.updateProject);
  const deleteRun = useStore((s) => s.deleteRun);
  const loadRecentProjects = useStore((s) => s.loadRecentProjects);

  const [newName, setNewName] = useState("New rocket project");
  const [searchRoot, setSearchRoot] = useState("");
  const [confirmDelete, setConfirmDelete] = useState(true);
  const [nameDraft, setNameDraft] = useState(project?.name ?? "");
  const [descriptionDraft, setDescriptionDraft] = useState(project?.description ?? "");

  useEffect(() => {
    void useStore.getState().refreshProject();
    void loadRecentProjects();
  }, [loadRecentProjects]);

  useEffect(() => {
    setNameDraft(project?.name ?? "");
    setDescriptionDraft(project?.description ?? "");
  }, [project?.name, project?.description]);

  const handleCreate = async () => {
    const directory = await pickDirectory("Choose an empty folder for the new project");
    if (!directory) return;
    await createProject(directory, newName.trim() || "Untitled project");
  };

  const handleOpen = async () => {
    const directory = await pickDirectory("Choose a HexaDOF project folder");
    if (!directory) return;
    await openProject(directory);
  };

  const handleDiscover = async () => {
    const directory = await pickDirectory("Choose a folder to search for projects");
    if (!directory) return;
    setSearchRoot(directory);
    await discoverProjects(directory);
  };

  return (
    <div className="page">
      <div>
        <h1 className="page-title">Projects</h1>
        <p className="page-subtitle">
          A project is a directory holding the imported models, saved runs, telemetry sessions, flight
          logs, and reports for one vehicle or campaign. Every artifact is a readable file, and saved
          runs are never overwritten.
        </p>
      </div>

      <div className="grid-12">
        <section className="col-6">
          <Panel title="Create a project" subtitle="A new directory with the full layout">
            <Field label="Project name" hint="Shown in the top bar and used for the project file.">
              <TextInput value={newName} onChange={(e) => setNewName(e.target.value)} />
            </Field>
            <Button
              variant="primary"
              icon={<IconPlus />}
              onClick={() => void handleCreate()}
              disabled={busy !== null}
            >
              Choose a folder and create
            </Button>
            <Hint>
              The folder must be empty. HexaDOF creates models, simulations, telemetry, flights, reports,
              and assets inside it.
            </Hint>
          </Panel>

          <Panel title="Open a project">
            <Button icon={<IconOpen />} onClick={() => void handleOpen()} disabled={busy !== null}>
              Choose a project folder
            </Button>
            {recent.length > 0 && (
              <>
                <div className="divider" />
                <span className="meta">Recent projects</span>
                <div className="col" style={{ gap: "var(--space-1)" }}>
                  {recent.map((path) => (
                    <button
                      key={path}
                      type="button"
                      className="palette-item"
                      onClick={() => void openProject(path)}
                      title={path}
                    >
                      <IconProjects />
                      <span className="truncate">{path}</span>
                    </button>
                  ))}
                </div>
              </>
            )}
          </Panel>

          <Panel title="Find projects" subtitle={searchRoot || "Search a folder tree"}>
            <Button onClick={() => void handleDiscover()}>Choose a folder to search</Button>
            {discovered.length > 0 && (
              <DataTable
                headers={[
                  { label: "Project" },
                  { label: "Folder" },
                  { label: "Updated" },
                  { label: "" },
                ]}
              >
                {discovered.map((entry) => (
                  <tr key={entry.root}>
                    <td>
                      {entry.readable ? (
                        entry.name
                      ) : (
                        <Badge tone="error">Unreadable</Badge>
                      )}
                    </td>
                    <td className="truncate" title={entry.root}>
                      {entry.root}
                    </td>
                    <td>{entry.readable ? formatRelative(entry.updated_at) : entry.problem}</td>
                    <td>
                      <Button
                        size="small"
                        onClick={() => void openProject(entry.root)}
                        disabled={!entry.readable}
                      >
                        Open
                      </Button>
                    </td>
                  </tr>
                ))}
              </DataTable>
            )}
            {discovered.length === 0 && searchRoot && (
              <Notice
                level="info"
                title="No projects found"
                detail="A project folder contains a project.hexadof.json file. If the project is elsewhere, search a different folder."
              />
            )}
          </Panel>
        </section>

        <section className="col-6">
          {project ? (
            <>
              <Panel
                title="Current project"
                subtitle={project.root}
                actions={
                  <Button size="small" onClick={() => void saveProject()}>
                    Save
                  </Button>
                }
              >
                {project.warnings.length > 0 && (
                  <Notice
                    level="warning"
                    title="Notes from the last open"
                    detail={project.warnings.join(" ")}
                  />
                )}
                <div className="field-row">
                  <Field label="Name">
                    <TextInput
                      value={nameDraft}
                      onChange={(e) => setNameDraft(e.target.value)}
                      onBlur={() => {
                        if (nameDraft !== project.name) void updateProject(nameDraft, undefined);
                      }}
                    />
                  </Field>
                  <Field label="Folder">
                    <TextInput value={project.root} readOnly />
                  </Field>
                </div>
                <Field label="Description">
                  <TextInput
                    value={descriptionDraft}
                    placeholder="What is this project for?"
                    onChange={(e) => setDescriptionDraft(e.target.value)}
                    onBlur={() => {
                      if (descriptionDraft !== project.description) {
                        void updateProject(undefined, descriptionDraft);
                      }
                    }}
                  />
                </Field>
                <dl className="kv">
                  <dt>Identifier</dt>
                  <dd>{project.project_id}</dd>
                  <dt>Schema version</dt>
                  <dd>{project.schema_version}</dd>
                  <dt>Units</dt>
                  <dd>{project.unit_system}</dd>
                  <dt>World frame</dt>
                  <dd>{project.world_frame}</dd>
                  <dt>Body frame</dt>
                  <dd>{project.body_frame}</dd>
                  <dt>Created</dt>
                  <dd>{formatTimestamp(project.created_at)}</dd>
                  <dt>Updated</dt>
                  <dd>{formatTimestamp(project.updated_at)}</dd>
                  <dt>Model reference</dt>
                  <dd>{project.model_reference ?? "none"}</dd>
                  <dt>Layout complete</dt>
                  <dd>{project.layout_complete ? "yes" : "no, folders were recreated"}</dd>
                </dl>
              </Panel>

              <Panel title="Contents">
                <DataTable
                  headers={[
                    { label: "Kind" },
                    { label: "Count", numeric: true },
                    { label: "Folder" },
                  ]}
                >
                  <tr>
                    <td>Dynamics models</td>
                    <td className="num">{project.inventory.models}</td>
                    <td className="mono">models/</td>
                  </tr>
                  <tr>
                    <td>Simulation runs</td>
                    <td className="num">{project.inventory.simulations}</td>
                    <td className="mono">simulations/</td>
                  </tr>
                  <tr>
                    <td>Flight logs</td>
                    <td className="num">{project.inventory.flight_logs}</td>
                    <td className="mono">flights/</td>
                  </tr>
                  <tr>
                    <td>Telemetry sessions</td>
                    <td className="num">{project.inventory.telemetry_sessions}</td>
                    <td className="mono">telemetry/</td>
                  </tr>
                  <tr>
                    <td>Reports</td>
                    <td className="num">{project.inventory.reports}</td>
                    <td className="mono">reports/</td>
                  </tr>
                </DataTable>
                <Hint>
                  Deleting a saved run or session is permanent. HexaDOF always asks first, and never
                  removes anything on its own.
                </Hint>
                <Checkbox
                  label="Ask before deleting an artifact"
                  checked={confirmDelete}
                  onChange={setConfirmDelete}
                />
              </Panel>
            </>
          ) : (
            <Panel title="Current project">
              <EmptyState
                icon={<IconProjects />}
                title="No project is open"
                detail="Create a project to import a dynamics model, run a simulation, and store the results. Everything HexaDOF writes lives inside the project directory."
              />
            </Panel>
          )}

          <Panel title="Saved runs" subtitle="Open or remove a run">
            <SavedRuns confirmDelete={confirmDelete} onDelete={(name) => void deleteRun(name)} />
          </Panel>
        </section>
      </div>
    </div>
  );
}

function SavedRuns({
  confirmDelete,
  onDelete,
}: {
  confirmDelete: boolean;
  onDelete: (name: string) => void;
}) {
  const savedRuns = useStore((s) => s.savedRuns);
  const openSavedRun = useStore((s) => s.openSavedRun);

  if (savedRuns.length === 0) {
    return (
      <EmptyState
        title="No saved runs"
        detail="A run is saved from the Dynamics screen. Saved runs are immutable, so a re-run always produces a new directory."
      />
    );
  }

  return (
    <DataTable
      headers={[
        { label: "Run" },
        { label: "Status" },
        { label: "Samples", numeric: true },
        { label: "Duration", numeric: true },
        { label: "" },
      ]}
    >
      {savedRuns.map((run) => (
        <tr key={run.name}>
          <td>
            <div className="col" style={{ gap: 0 }}>
              <span>{run.label}</span>
              <span className="meta mono">{run.name}</span>
            </div>
          </td>
          <td>{run.status}</td>
          <td className="num">{run.samples}</td>
          <td className="num">{run.duration.toFixed(3)} s</td>
          <td>
            <div className="row">
              <Button size="small" onClick={() => void openSavedRun(run.name)}>
                Open
              </Button>
              <Button
                size="small"
                variant="danger"
                icon={<IconTrash />}
                onClick={() => {
                  if (!confirmDelete || window.confirm(`Delete ${run.name}? This cannot be undone.`)) {
                    onDelete(run.name);
                  }
                }}
                aria-label={`Delete ${run.name}`}
              />
            </div>
          </td>
        </tr>
      ))}
    </DataTable>
  );
}
