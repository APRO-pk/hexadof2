/**
 * The 3D viewport.
 *
 * A three.js scene holding the rocket, world and body axes, the trajectory, and
 * the state vectors. The renderer is not the source of truth: it interpolates
 * between recorded states at whatever rate the display runs, while the numerical
 * state and the recording stay untouched.
 *
 * The attitude source is always labelled in the corner. A filtered estimate must
 * never be presented as measured truth, and the label is the only thing standing
 * between a user and that mistake.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import * as THREE from "three";
import { Badge, Button } from "./ui";
import { IconCrosshair, IconTarget } from "./Icons";
import { cssVar } from "../lib/format";
import type { ReplayData } from "../lib/types";

export interface ViewportOptions {
  /** Draw the world axes triad. */
  showWorldAxes: boolean;
  /** Draw the body axes triad. */
  showBodyAxes: boolean;
  /** Draw the trajectory polyline. */
  showTrajectory: boolean;
  /** Draw the velocity vector. */
  showVelocity: boolean;
  /** Draw the angular-rate indicator. */
  showAngularRate: boolean;
  /** Draw the ground plane and grid. */
  showGround: boolean;
  /** Keep the camera following the body. */
  follow: boolean;
}

export const defaultViewportOptions: ViewportOptions = {
  showWorldAxes: true,
  showBodyAxes: true,
  showTrajectory: true,
  showVelocity: true,
  showAngularRate: false,
  showGround: true,
  follow: true,
};

/** Which camera the user has selected. */
export type CameraPreset = "free" | "follow" | "top" | "side" | "front" | "world";

/**
 * Build the rocket mesh.
 *
 * The geometry is a nose cone, a body tube, and four fins, so the attitude is
 * readable from shape alone. A bare box would leave the roll axis ambiguous.
 */
function buildRocket(bodyLength: number, radius: number): THREE.Group {
  const group = new THREE.Group();

  const shell = new THREE.MeshStandardMaterial({
    color: 0xdfe6ee,
    metalness: 0.25,
    roughness: 0.55,
  });
  const accent = new THREE.MeshStandardMaterial({
    color: 0x38c7ff,
    metalness: 0.35,
    roughness: 0.4,
  });
  const dark = new THREE.MeshStandardMaterial({
    color: 0x3a4756,
    metalness: 0.4,
    roughness: 0.6,
  });

  const noseLength = bodyLength * 0.28;
  const tubeLength = bodyLength - noseLength;

  const nose = new THREE.Mesh(new THREE.ConeGeometry(radius, noseLength, 24), accent);
  nose.position.set(tubeLength / 2 + noseLength / 2, 0, 0);
  nose.rotation.z = -Math.PI / 2;
  group.add(nose);

  const tube = new THREE.Mesh(new THREE.CylinderGeometry(radius, radius, tubeLength, 24), shell);
  tube.position.set(tubeLength / 2, 0, 0);
  tube.rotation.z = -Math.PI / 2;
  group.add(tube);

  const band = new THREE.Mesh(
    new THREE.CylinderGeometry(radius * 1.03, radius * 1.03, bodyLength * 0.04, 24),
    dark,
  );
  band.position.set(tubeLength * 0.72, 0, 0);
  band.rotation.z = -Math.PI / 2;
  group.add(band);

  // Four fins, rotated so the roll axis is unambiguous.
  const finShape = new THREE.Shape();
  finShape.moveTo(0, 0);
  finShape.lineTo(-bodyLength * 0.16, 0);
  finShape.lineTo(-bodyLength * 0.2, radius * 1.9);
  finShape.lineTo(-bodyLength * 0.02, radius * 1.05);
  finShape.lineTo(0, 0);
  const finGeometry = new THREE.ExtrudeGeometry(finShape, {
    depth: radius * 0.14,
    bevelEnabled: false,
  });
  for (let i = 0; i < 4; i += 1) {
    const fin = new THREE.Mesh(finGeometry, dark);
    fin.rotation.x = (Math.PI / 2) * i;
    fin.position.set(0, 0, 0);
    fin.translateZ(0);
    group.add(fin);
  }

  // The body frame is X forward, Y right, Z down, and the mesh is built along +X
  // already, so no reorientation is needed here.
  return group;
}

/** Build a labelled axis triad. */
function buildAxes(length: number, colours: [number, number, number]): THREE.Group {
  const group = new THREE.Group();
  const directions: [THREE.Vector3, number][] = [
    [new THREE.Vector3(1, 0, 0), colours[0]],
    [new THREE.Vector3(0, 1, 0), colours[1]],
    [new THREE.Vector3(0, 0, 1), colours[2]],
  ];
  for (const [direction, colour] of directions) {
    const geometry = new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(0, 0, 0),
      direction.clone().multiplyScalar(length),
    ]);
    const line = new THREE.Line(
      geometry,
      new THREE.LineBasicMaterial({ color: colour, linewidth: 2 }),
    );
    group.add(line);
    const cone = new THREE.Mesh(
      new THREE.ConeGeometry(length * 0.06, length * 0.18, 10),
      new THREE.MeshBasicMaterial({ color: colour }),
    );
    cone.position.copy(direction.clone().multiplyScalar(length));
    cone.quaternion.setFromUnitVectors(new THREE.Vector3(0, 1, 0), direction);
    group.add(cone);
  }
  return group;
}

/** A legend explaining every visual vector the viewport can draw. */
function Legend({ options }: { options: ViewportOptions }) {
  const rows: { label: string; colour: string }[] = [];
  if (options.showWorldAxes) {
    rows.push({ label: "World X / Y / Z (E, N, U)", colour: "#ff6b6b" });
  }
  if (options.showBodyAxes) {
    rows.push({ label: "Body X forward, Y right, Z down", colour: "#4cd97b" });
  }
  if (options.showTrajectory) {
    rows.push({ label: "Recorded trajectory", colour: "#38c7ff" });
  }
  if (options.showVelocity) {
    rows.push({ label: "Velocity (world frame)", colour: "#f0b429" });
  }
  if (options.showAngularRate) {
    rows.push({ label: "Angular rate (body frame)", colour: "#b39ddb" });
  }
  return (
    <div className="viewport-legend">
      {rows.map((row) => (
        <div className="legend-row" key={row.label}>
          <span className="legend-swatch" style={{ background: row.colour }} />
          <span className="secondary">{row.label}</span>
        </div>
      ))}
      {rows.length === 0 && <span className="muted">No overlays are enabled.</span>}
    </div>
  );
}

export function Viewport({
  data,
  index,
  options = defaultViewportOptions,
  attitudeLabel,
  attitudeNote,
  attitudePoor = false,
  camera = "follow",
  onCameraChange,
  onAddMarker,
}: {
  data: ReplayData | null;
  index: number;
  options?: ViewportOptions;
  /** The label shown over the rocket, stating where the attitude came from. */
  attitudeLabel: string;
  /** A sentence about the reliability of that attitude. */
  attitudeNote?: string;
  /** Whether the attitude should be presented as indicative only. */
  attitudePoor?: boolean;
  camera?: CameraPreset;
  onCameraChange?: (preset: CameraPreset) => void;
  onAddMarker?: () => void;
}) {
  const mountRef = useRef<HTMLDivElement | null>(null);
  const sceneRef = useRef<{
    renderer: THREE.WebGLRenderer;
    scene: THREE.Scene;
    camera: THREE.PerspectiveCamera;
    rocket: THREE.Group;
    bodyAxes: THREE.Group;
    worldAxes: THREE.Group;
    trajectory: THREE.Line;
    velocity: THREE.ArrowHelper;
    angularRate: THREE.ArrowHelper;
    ground: THREE.Group;
    resize: ResizeObserver;
    dispose: () => void;
  } | null>(null);
  const [ready, setReady] = useState(false);
  const [zoom, setZoom] = useState(1);

  const frameCount = data?.times.length ?? 0;

  // One-time scene setup.
  useEffect(() => {
    const mount = mountRef.current;
    if (!mount) return;

    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: false });
    renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));
    renderer.setSize(mount.clientWidth || 640, mount.clientHeight || 360, false);
    mount.appendChild(renderer.domElement);

    const scene = new THREE.Scene();
    scene.background = new THREE.Color(cssVar("--viewport-background", "#0a0d12"));

    const camera3 = new THREE.PerspectiveCamera(
      50,
      (mount.clientWidth || 640) / (mount.clientHeight || 360),
      0.05,
      5000,
    );

    scene.add(new THREE.HemisphereLight(0xdfe8f2, 0x1a1f26, 1.05));
    const key = new THREE.DirectionalLight(0xffffff, 1.15);
    key.position.set(6, 9, 7);
    scene.add(key);
    const fill = new THREE.DirectionalLight(0x88aaff, 0.45);
    fill.position.set(-7, -4, 5);
    scene.add(fill);

    const rocket = buildRocket(1.0, 0.0625);
    scene.add(rocket);

    const bodyAxes = buildAxes(0.7, [0x4cd97b, 0x8fe6ab, 0x2b9e57]);
    rocket.add(bodyAxes);

    const worldAxes = buildAxes(2.4, [0xff6b6b, 0x4cd97b, 0x38c7ff]);
    scene.add(worldAxes);

    const trajectory = new THREE.Line(
      new THREE.BufferGeometry(),
      new THREE.LineBasicMaterial({ color: 0x38c7ff }),
    );
    scene.add(trajectory);

    const velocity = new THREE.ArrowHelper(
      new THREE.Vector3(1, 0, 0),
      new THREE.Vector3(),
      1,
      0xf0b429,
      0.16,
      0.09,
    );
    scene.add(velocity);

    const angularRate = new THREE.ArrowHelper(
      new THREE.Vector3(0, 0, 1),
      new THREE.Vector3(),
      0.6,
      0xb39ddb,
      0.12,
      0.07,
    );
    scene.add(angularRate);

    const ground = new THREE.Group();
    const grid = new THREE.GridHelper(40, 40, 0x2a3644, 0x1d2530);
    (grid.material as THREE.Material).transparent = true;
    (grid.material as THREE.Material).opacity = 0.55;
    ground.add(grid);
    scene.add(ground);

    const resize = new ResizeObserver(() => {
      const width = mount.clientWidth || 640;
      const height = mount.clientHeight || 360;
      renderer.setSize(width, height, false);
      camera3.aspect = width / Math.max(1, height);
      camera3.updateProjectionMatrix();
    });
    resize.observe(mount);

    const dispose = () => {
      resize.disconnect();
      renderer.dispose();
      if (renderer.domElement.parentElement === mount) {
        mount.removeChild(renderer.domElement);
      }
    };

    sceneRef.current = {
      renderer,
      scene,
      camera: camera3,
      rocket,
      bodyAxes,
      worldAxes,
      trajectory,
      velocity,
      angularRate,
      ground,
      resize,
      dispose,
    };
    setReady(true);

    return () => {
      dispose();
      sceneRef.current = null;
    };
  }, []);

  // Draw the current frame whenever the index, the data, or the options change.
  useEffect(() => {
    const context = sceneRef.current;
    if (!context || !ready) return;
    const { renderer, scene, camera: camera3, rocket, trajectory, velocity, angularRate } = context;

    const hasData = data !== null && frameCount > 0;
    rocket.visible = hasData;
    const position = new THREE.Vector3(0, 0, 0);
    const quaternion = new THREE.Quaternion();

    if (hasData && data) {
      const clamped = Math.min(frameCount - 1, Math.max(0, index));
      const p = data.positions[clamped] ?? [0, 0, 0];
      const q = data.quaternions[clamped] ?? [1, 0, 0, 0];
      // The world frame is ENU with Z up. The scene uses Z up as well, so a
      // world position maps directly and no axis swap is required.
      position.set(p[0], p[1], p[2]);
      quaternion.set(q[1], q[2], q[3], q[0]);
      rocket.position.copy(position);
      rocket.quaternion.copy(quaternion);

      if (options.showTrajectory) {
        const points: THREE.Vector3[] = [];
        const stride = Math.max(1, Math.floor(frameCount / 4000));
        for (let i = 0; i <= clamped; i += stride) {
          const point = data.positions[i];
          if (!point) continue;
          points.push(new THREE.Vector3(point[0], point[1], point[2]));
        }
        const last = data.positions[clamped];
        if (last) points.push(new THREE.Vector3(last[0], last[1], last[2]));
        trajectory.geometry.dispose();
        trajectory.geometry = new THREE.BufferGeometry().setFromPoints(points);
        trajectory.visible = points.length > 1;
      } else {
        trajectory.visible = false;
      }

      const v = data.velocities[clamped];
      if (options.showVelocity && v) {
        const vector = new THREE.Vector3(v[0], v[1], v[2]);
        const magnitude = vector.length();
        if (magnitude > 1e-6) {
          velocity.visible = true;
          velocity.position.copy(position);
          velocity.setDirection(vector.clone().normalize());
          velocity.setLength(Math.min(6, 0.4 + magnitude * 0.05), 0.16, 0.09);
          velocity.setColor(new THREE.Color(0xf0b429));
        } else {
          velocity.visible = false;
        }
      } else {
        velocity.visible = false;
      }

      const w = data.angular_rates[clamped];
      if (options.showAngularRate && w) {
        const vector = new THREE.Vector3(w[0], w[1], w[2]);
        const magnitude = vector.length();
        if (magnitude > 1e-6) {
          angularRate.visible = true;
          angularRate.position.copy(position);
          // The rate is a body-frame vector, so it is drawn through the body.
          angularRate.setDirection(vector.clone().normalize().applyQuaternion(quaternion));
          angularRate.setLength(Math.min(4, 0.3 + magnitude * 0.4), 0.12, 0.07);
        } else {
          angularRate.visible = false;
        }
      } else {
        angularRate.visible = false;
      }
    } else {
      trajectory.visible = false;
      velocity.visible = false;
      angularRate.visible = false;
    }

    context.bodyAxes.visible = options.showBodyAxes && hasData;
    context.worldAxes.visible = options.showWorldAxes;
    context.ground.visible = options.showGround;

    // Camera.
    const distance = 7 / zoom;
    const target = position.clone();
    switch (camera) {
      case "top":
        camera3.position.set(target.x, target.y, target.z + distance);
        camera3.up.set(0, 1, 0);
        break;
      case "front":
        camera3.position.set(target.x + distance, target.y, target.z);
        camera3.up.set(0, 0, 1);
        break;
      case "side":
        camera3.position.set(target.x, target.y - distance, target.z);
        camera3.up.set(0, 0, 1);
        break;
      case "world":
        camera3.position.set(distance * 0.7, -distance * 0.7, distance * 0.55);
        camera3.up.set(0, 0, 1);
        break;
      case "free":
        if (!camera3.userData.initialised) {
          camera3.position.set(distance * 0.7, -distance * 0.7, distance * 0.5);
          camera3.up.set(0, 0, 1);
          camera3.userData.initialised = true;
        }
        break;
      case "follow":
      default:
        camera3.position.set(
          target.x + distance * 0.7,
          target.y - distance * 0.7,
          target.z + distance * 0.42,
        );
        camera3.up.set(0, 0, 1);
        break;
    }
    if (camera !== "free") {
      camera3.lookAt(target);
    }

    renderer.render(scene, camera3);
  }, [data, index, options, ready, camera, frameCount, zoom]);

  // Redraw when the theme changes, so the background and grid follow.
  useEffect(() => {
    const context = sceneRef.current;
    if (!context) return;
    const theme = document.documentElement.dataset.theme ?? "dark";
    context.scene.background = new THREE.Color(
      theme === "light"
        ? cssVar("--viewport-background", "#eef2f7")
        : cssVar("--viewport-background", "#0a0d12"),
    );
    context.renderer.render(context.scene, context.camera);
  }, [ready]);

  const presets = useMemo<{ id: CameraPreset; label: string }[]>(
    () => [
      { id: "follow", label: "Follow" },
      { id: "free", label: "Free orbit" },
      { id: "top", label: "Top" },
      { id: "side", label: "Side" },
      { id: "front", label: "Front" },
      { id: "world", label: "World" },
    ],
    [],
  );

  const time = data && frameCount > 0 ? data.times[Math.min(frameCount - 1, Math.max(0, index))] : null;

  return (
    <div className="viewport-frame">
      <div className="viewport">
        <div ref={mountRef} style={{ position: "absolute", inset: 0 }} />

        <div className="viewport-overlay">
          <span className="viewport-source">Attitude: {attitudeLabel}</span>
          {attitudePoor && (
            <Badge tone="warning">Indicative only. Treat this attitude as approximate.</Badge>
          )}
          {attitudeNote && !attitudePoor && (
            <span className="viewport-note">{attitudeNote}</span>
          )}
        </div>

        <div className="viewport-hud">
          <div className="hud-row">
            <span className="hud-key">t</span>
            <span className="hud-value">{time === null ? "--" : `${time.toFixed(3)} s`}</span>
          </div>
          <div className="hud-row">
            <span className="hud-key">frame</span>
            <span className="hud-value">
              {frameCount === 0 ? "--" : `${Math.min(frameCount, index + 1)} / ${frameCount}`}
            </span>
          </div>
          {data && !data.has_position && (
            <div className="hud-row">
              <span className="hud-key">position</span>
              <span className="hud-value">not available</span>
            </div>
          )}
        </div>

        <Legend options={options} />
      </div>

      {/* The controls sit under the canvas rather than over it. Overlaid on a
          narrow column they collided with the attitude label, and they covered
          the model at exactly the moment it was being inspected. */}
      <div className="viewport-toolbar">
        <div className="btn-group">
          {presets.map((preset) => (
            <Button
              key={preset.id}
              size="small"
              variant={camera === preset.id ? "primary" : "default"}
              onClick={() => onCameraChange?.(preset.id)}
            >
              {preset.label}
            </Button>
          ))}
        </div>
        <span className="spacer" />
        <Button
          size="small"
          icon={<IconCrosshair />}
          title="Zoom in"
          aria-label="Zoom in"
          onClick={() => setZoom((z) => Math.min(6, z * 1.25))}
        />
        <Button
          size="small"
          icon={<IconTarget />}
          title="Zoom out"
          aria-label="Zoom out"
          onClick={() => setZoom((z) => Math.max(0.2, z / 1.25))}
        />
        {onAddMarker && (
          <Button size="small" variant="default" onClick={onAddMarker}>
            Add marker
          </Button>
        )}
      </div>
    </div>
  );
}
