/**
 * The example flights the Dynamics screen offers.
 *
 * Every one of these is a complete, documented scenario: a vehicle, a motor, an
 * environment, and a reason to look at it. They exist so a new reader can run
 * something that flies in one press, and so the interesting parts of the tool
 * (a landing burn, a spin, a crosswind, a pure coast) each have a case that
 * shows them without anybody having to invent numbers first.
 *
 * The numbers are honest for the built-in example model: a 4.5 kg rocket with a
 * 125 mm body. Change the mass or the thrust and the flight changes, which is
 * the point of a simulation.
 */

import type { ScenarioRequest } from "./types";
import { defaultPlotChannels, defaultScenario } from "./store";

/** One example flight, as the picker lists it. */
export interface ExampleFlight {
  id: string;
  name: string;
  /** One line for the list. */
  summary: string;
  /** What to look at once it has run. */
  detail: string;
  /** The channels worth plotting for this case. */
  channels: string[];
  /** The scenario the run is configured with. */
  scenario: ScenarioRequest;
}

/** The example model's numbers, so every example agrees with it. */
const MASS = 4.5;
const INERTIA: [number, number, number, number, number, number] = [0.09, 0.09, 0.012, 0, 0, 0];
/** Nose up: -90 degrees of pitch points body X along world +Z. */
const NOSE_UP: [number, number, number] = [0, -Math.PI / 2, 0];

function base(): ScenarioRequest {
  return {
    ...defaultScenario(),
    mass: MASS,
    inertia: INERTIA,
    euler: NOSE_UP,
    angular_velocity: [0, 0, 0],
    standard_atmosphere: true,
    gravity_enabled: true,
    gravity: 9.80665,
    wind_speed: 0,
    ground_elevation: 0,
    lift_slope: 2,
    pitch_damping: -8,
    control_moment: null,
    landing_burn: null,
    save: false,
  };
}

export const EXAMPLE_FLIGHTS: ExampleFlight[] = [
  {
    id: "sounding_rocket",
    name: "Sounding rocket",
    summary: "A 4.5 kg rocket, 180 N for 2.4 s from a 1.5 m rail.",
    detail:
      "The plain case: boost, coast to apogee, and a ballistic descent to the ground. It is the flight the timeline phases were drawn for, and the drag coefficient is the only aerodynamic input.",
    channels: defaultPlotChannels(),
    scenario: {
      ...base(),
      name: "Example sounding rocket",
      description:
        "A 4.5 kg sounding rocket on a 1.5 m vertical rail: 180 N for 2.4 s, drag coefficient 0.45 against a 125 mm body.",
      end_time: 20,
      output_interval: 0.02,
      rail_length: 1.5,
      thrust: 180,
      burn_time: 2.4,
      drag_coefficient: 0.45,
      aero_source: "auto",
    },
  },
  {
    id: "starhopper",
    name: "Starhopper hop",
    summary: "A short hard burn, a low apogee, and a powered landing.",
    detail:
      "The engine lights again on the way down and the vehicle arrives at a few metres per second instead of tens. Watch the force channel: the landing pulse is a second step in thrust, and the landing burn is marked on the timeline.",
    channels: [
      "altitude",
      "vertical_velocity",
      "speed",
      "acceleration",
      "force",
      "mass",
      "dynamic_pressure",
    ],
    scenario: {
      ...base(),
      name: "Starhopper hop",
      description:
        "A short hop: 140 N for 1.6 s, then a landing burn that ignites on the stopping-distance condition.",
      end_time: 40,
      output_interval: 0.02,
      rail_length: 0,
      thrust: 140,
      burn_time: 1.6,
      drag_coefficient: 0.45,
      aero_source: "auto",
      landing_burn: {
        thrust: 90,
        deceleration_margin: 1,
        maximum_ignition_altitude: 120,
        minimum_descent_rate: 0.2,
      },
    },
  },
  {
    id: "hover_slam",
    name: "Self landing hover slam",
    summary: "A higher apogee, a long fall, and a landing burn that takes the speed out.",
    detail:
      "The burn only just has the thrust to stop the vehicle, so it lights late and low. Run the same scenario with the landing burn removed to see the difference the trigger makes.",
    channels: [
      "altitude",
      "vertical_velocity",
      "speed",
      "acceleration",
      "force",
      "dynamic_pressure",
    ],
    scenario: {
      ...base(),
      name: "Self landing hover slam",
      description:
        "220 N for 2.6 s to a high apogee, then a long fall arrested by a landing burn on the stopping-distance condition.",
      end_time: 60,
      output_interval: 0.01,
      rail_length: 0,
      thrust: 220,
      burn_time: 2.6,
      drag_coefficient: 0.45,
      aero_source: "auto",
      landing_burn: {
        thrust: 110,
        deceleration_margin: 1.5,
        maximum_ignition_altitude: 400,
        minimum_descent_rate: 0.2,
      },
    },
  },
  {
    id: "spin_stabilised",
    name: "Spin stabilised",
    summary: "The same rocket spun up about its roll axis before launch.",
    detail:
      "Shows the attitude channels doing something: a constant roll rate through the flight, with the pitch and yaw rates staying near zero because nothing disturbs them. No aerodynamic moments are modelled, so the spin is preserved exactly.",
    channels: [
      "altitude",
      "vertical_velocity",
      "roll",
      "pitch",
      "yaw",
      "angular_rate_x",
      "angular_rate_y",
      "angular_rate_z",
    ],
    scenario: {
      ...base(),
      name: "Spin stabilised flight",
      description:
        "A sounding rocket released with 6 rad/s of roll rate, to show the attitude channels over a whole flight.",
      end_time: 20,
      output_interval: 0.02,
      rail_length: 0,
      thrust: 180,
      burn_time: 2.4,
      drag_coefficient: 0.45,
      aero_source: "auto",
      angular_velocity: [6, 0, 0],
    },
  },
  {
    id: "crosswind",
    name: "Crosswind launch",
    summary: "An 8 m/s wind across a vertical flight.",
    detail:
      "The rocket still points up but the air does not, so it drifts downwind and flies at a small angle of attack and sideslip. Read the two aerodynamic angles together with the horizontal position channels.",
    channels: [
      "altitude",
      "vertical_velocity",
      "angle_of_attack",
      "sideslip",
      "dynamic_pressure",
      "speed",
    ],
    scenario: {
      ...base(),
      name: "Crosswind launch",
      description:
        "A vertical flight in an 8 m/s crosswind, to show the aerodynamic angles and the drift it causes.",
      end_time: 20,
      output_interval: 0.02,
      rail_length: 1.5,
      thrust: 180,
      burn_time: 2.4,
      drag_coefficient: 0.45,
      aero_source: "auto",
      wind_speed: 8,
      wind_direction: [1, 0, 0],
    },
  },
  {
    id: "drop_test",
    name: "Drop test",
    summary: "No motor at all: released at 300 m and left to fall.",
    detail:
      "The pure dynamics case, useful for checking drag and the ground plane without a motor in the way. Energy drift is reported on a run like this one, because nothing is adding energy.",
    channels: ["altitude", "vertical_velocity", "speed", "acceleration", "force", "dynamic_pressure"],
    scenario: {
      ...base(),
      name: "Drop test",
      description:
        "A 4.5 kg body released at 300 m with no thrust, falling to the ground under gravity and drag.",
      end_time: 30,
      output_interval: 0.02,
      rail_length: 0,
      position: [0, 0, 300],
      velocity: [0, 0, 0],
      thrust: null,
      burn_time: null,
      drag_coefficient: 0.45,
      aero_source: "estimate",
    },
  },
];

/** Look one example up by id. */
export function exampleFlight(id: string): ExampleFlight | undefined {
  return EXAMPLE_FLIGHTS.find((example) => example.id === id);
}
