import { ButtonItem, MenuGroup, Section } from "@atlaskit/menu";
import ModalDialog, { ModalBody, ModalHeader, ModalTitle, ModalTransition } from "@atlaskit/modal-dialog";
import { Text } from "@atlaskit/primitives/compiled";
import { useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useAlertFeed } from "../api/useAlertFeed";
import { TextInput, useRerenderOnMount } from "../ui/primitives";

export type PaletteScreen = { to: string; label: string };
type Result = { key: string; label: string; hint: string; to: string };

const MAX_ALERTS = 8;

function PaletteDialog({ screens, onClose, onClosed }: { screens: PaletteScreen[]; onClose: () => void; onClosed: () => void }) {
  const navigate = useNavigate();
  const { alerts } = useAlertFeed();
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  useRerenderOnMount(() => inputRef.current?.focus());

  const q = query.trim().toLowerCase();
  const results: Result[] = [
    ...screens
      .filter((s) => s.label.toLowerCase().includes(q))
      .map((s) => ({ key: `screen:${s.to}`, label: s.label, hint: "Ekran", to: s.to })),
    ...(q
      ? alerts
          .filter((a) => a.title.toLowerCase().includes(q))
          .slice(0, MAX_ALERTS)
          .map((a) => ({ key: `alert:${a.id}`, label: a.title, hint: "Alert", to: `/alerts/${a.id}` }))
      : []),
  ];
  const current = Math.min(active, results.length - 1);

  const go = (r: Result | undefined) => {
    if (!r) return;
    onClose();
    navigate(r.to);
  };

  return (
    <ModalDialog onClose={onClose} onCloseComplete={onClosed} width="small" autoFocus={inputRef}>
      <ModalHeader hasCloseButton>
        <ModalTitle>Paleta komend</ModalTitle>
      </ModalHeader>
      <ModalBody>
        <TextInput
          ref={inputRef}
          aria-label="Szukaj ekranu lub alertu"
          placeholder="Ekran albo tytuł alertu…"
          value={query}
          onChange={(e) => {
            setQuery(e.currentTarget.value);
            setActive(0);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown" || e.key === "ArrowUp") {
              e.preventDefault();
              setActive(Math.max(0, Math.min(results.length - 1, current + (e.key === "ArrowDown" ? 1 : -1))));
            } else if (e.key === "Enter") {
              e.preventDefault();
              go(results[current]);
            }
          }}
        />
        <MenuGroup>
          <Section title={q ? "Wyniki" : "Ekrany"}>
            {results.length === 0 && <Text as="p" color="color.text.subtle">Brak wyników.</Text>}
            {results.map((r, i) => (
              <ButtonItem key={r.key} isSelected={i === current} description={r.hint} onClick={() => go(r)}>
                {r.label}
              </ButtonItem>
            ))}
          </Section>
        </MenuGroup>
      </ModalBody>
    </ModalDialog>
  );
}

export default function CommandPalette({ screens }: { screens: PaletteScreen[] }) {
  const [open, setOpen] = useState(false);
  const openRef = useRef(false);
  const returnTo = useRef<HTMLElement | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey || e.key.toLowerCase() !== "k") return;
      e.preventDefault();
      if (openRef.current) {
        openRef.current = false;
        setOpen(false);
        return;
      }
      if (document.querySelector('[role="dialog"]')) return;
      returnTo.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      openRef.current = true;
      setOpen(true);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const close = () => {
    openRef.current = false;
    setOpen(false);
  };

  return (
    <ModalTransition>
      {open && <PaletteDialog screens={screens} onClose={close} onClosed={() => setTimeout(() => returnTo.current?.focus(), 0)} />}
    </ModalTransition>
  );
}
