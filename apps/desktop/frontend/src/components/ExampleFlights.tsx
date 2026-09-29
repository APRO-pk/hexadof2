/**
 * The example flight picker.
 *
 * The Configuration panel has one button for examples, because a screen full of
 * example buttons is a screen nobody reads. The button opens this list, and the
 * list is where the choice is explained: what the flight is, and what to look at
 * once it has run. Loading one installs the built-in model if there is none,
 * installs the scenario, and validates it.
 */

import { Modal, Badge, Button, Hint } from "./ui";
import { IconRocket } from "./Icons";
import { EXAMPLE_FLIGHTS, type ExampleFlight } from "../lib/examples";

export function ExampleFlightDialog({
  open,
  onClose,
  onLoad,
}: {
  open: boolean;
  onClose: () => void;
  onLoad: (id: string) => void | Promise<void>;
}) {
  if (!open) return null;

  return (
    <Modal
      title="Example flights"
      wide
      onClose={onClose}
      footer={
        <>
          <Hint>
            Each one installs a complete scenario for the built-in example model and validates it.
            Every number is editable afterwards, and the run records what it was given.
          </Hint>
          <span className="spacer" />
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
        </>
      }
    >
      <div className="example-list">
        {EXAMPLE_FLIGHTS.map((example) => (
          <ExampleCard key={example.id} example={example} onLoad={onLoad} />
        ))}
      </div>
    </Modal>
  );
}

function ExampleCard({
  example,
  onLoad,
}: {
  example: ExampleFlight;
  onLoad: (id: string) => void | Promise<void>;
}) {
  // A scenario with a landing burn is the one thing here that behaves in a way a
  // reader would not guess, so it is called out rather than left to the plots.
  const powered = example.scenario.landing_burn !== null && example.scenario.landing_burn !== undefined;
  const thrust = example.scenario.thrust ?? 0;

  return (
    <article className="example-card">
      <header className="example-card-head">
        <IconRocket className="example-card-icon" />
        <div className="grow">
          <h3 className="example-card-name">{example.name}</h3>
          <p className="example-card-summary">{example.summary}</p>
        </div>
        <div className="row" style={{ gap: "var(--space-2)" }}>
          {powered && <Badge tone="accent">Powered landing</Badge>}
          {thrust > 0 ? (
            <Badge tone="neutral">{thrust} N</Badge>
          ) : (
            <Badge tone="neutral">No motor</Badge>
          )}
          <Button size="small" variant="primary" onClick={() => void onLoad(example.id)}>
            Load
          </Button>
        </div>
      </header>
      <p className="example-card-detail">{example.detail}</p>
    </article>
  );
}
