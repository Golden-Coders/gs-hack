import { Checkbox } from "@atlaskit/checkbox";
import { Text } from "@atlaskit/primitives/compiled";
import { useState } from "react";
import { api } from "../api/client";
import { SeverityBadge } from "../components/Badges";
import { Card, PageHeader } from "../ui/primitives";
import "./MockOutbox.css";

type TierId = "critical" | "medium" | "other";
type ChannelId = "sms" | "phone" | "email" | "ui";

type TierConfig = {
  id: TierId;
  title: string;
  badge: "critical" | "warning" | "info";
  description: string;
};

const TIERS: TierConfig[] = [
  {
    id: "critical",
    title: "Critical",
    badge: "critical",
    description: "Immediate notifications for high-impact outages.",
  },
  {
    id: "medium",
    title: "Medium",
    badge: "warning",
    description: "Warnings and errors requiring team attention.",
  },
  {
    id: "other",
    title: "Other",
    badge: "info",
    description: "Informational and low-priority events.",
  },
];

const CHANNELS: { id: ChannelId; label: string }[] = [
  { id: "sms", label: "SMS" },
  { id: "phone", label: "Phone" },
  { id: "email", label: "Email" },
  { id: "ui", label: "Web UI" },
];

type Matrix = Record<TierId, Record<ChannelId, boolean>>;

const DEFAULT_MATRIX: Matrix = {
  critical: { sms: true, phone: true, email: false, ui: false },
  medium: { sms: false, phone: false, email: true, ui: false },
  other: { sms: false, phone: false, email: false, ui: true },
};

const STORAGE_KEY = "alerting.channelMatrix.v1";

function loadMatrix(): Matrix {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_MATRIX;
    const parsed = JSON.parse(raw) as Partial<Matrix>;
    return {
      critical: { ...DEFAULT_MATRIX.critical, ...parsed.critical },
      medium: { ...DEFAULT_MATRIX.medium, ...parsed.medium },
      other: { ...DEFAULT_MATRIX.other, ...parsed.other },
    };
  } catch {
    return DEFAULT_MATRIX;
  }
}

export default function MockOutbox() {
  const [matrix, setMatrix] = useState<Matrix>(loadMatrix);

  function toggle(tier: TierId, channel: ChannelId) {
    setMatrix((prev) => {
      const next: Matrix = {
        ...prev,
        [tier]: {
          ...prev[tier],
          [channel]: !prev[tier][channel],
        },
      };
      try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
      } catch {
      }
      if (channel !== "ui") {
        const anyEnabled = TIERS.some((t) => next[t.id][channel]);
        void api.PUT("/mock/channels/{channel}", {
          params: { path: { channel } },
          body: { available: anyEnabled },
        });
      }
      return next;
    });
  }

  return (
    <section className="mock-outbox">
      <PageHeader>Notification Channels</PageHeader>

      <Card title="Channel availability by alert severity">
        <ul className="tier-list" aria-label="Notification channels per alert severity">
          {TIERS.map((t) => (
            <li key={t.id} className="tier-card">
              <div className="tier-info">
                <div className="tier-header">
                  <SeverityBadge severity={t.badge} />
                  <Text as="strong" weight="bold">
                    {t.title}
                  </Text>
                </div>
                <Text size="small" color="color.text.subtle">
                  {t.description}
                </Text>
              </div>
              <div className="tier-channels">
                {CHANNELS.map((c) => (
                  <Checkbox
                    key={c.id}
                    label={c.label}
                    isChecked={matrix[t.id][c.id]}
                    onChange={() => toggle(t.id, c.id)}
                  />
                ))}
              </div>
            </li>
          ))}
        </ul>
      </Card>
    </section>
  );
}
