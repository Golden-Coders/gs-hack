import { forwardRef, useEffect, useId, useReducer, useRef, useState, type ReactElement, type ReactNode } from "react";
import { flushSync } from "react-dom";
import Button from "@atlaskit/button/new";
import ButtonGroup from "@atlaskit/button/button-group";
import { ErrorMessage, HelperMessage, Label } from "@atlaskit/form";
import EmptyState from "@atlaskit/empty-state";
import Heading from "@atlaskit/heading";
import Lozenge from "@atlaskit/lozenge";
import AdsPageHeader from "@atlaskit/page-header";
import { Inline, Stack, Text } from "@atlaskit/primitives/compiled";
import SectionMessage from "@atlaskit/section-message";
import AdsSelect from "@atlaskit/select";
import Skeleton from "@atlaskit/skeleton";
import Spinner from "@atlaskit/spinner";
import AdsTabs, { Tab, TabList, TabPanel } from "@atlaskit/tabs";
import AdsTextArea from "@atlaskit/textarea";
import Textfield from "@atlaskit/textfield";
import { token } from "@atlaskit/tokens";
import Tooltip from "@atlaskit/tooltip";
import { Copy } from "lucide-react";
import { adsIcon } from "./icons";
import { statusOf, type StatusDomain, type StatusValue } from "./status";

/** Ramka sekcji: nagłówek z akcjami, stan ładowania i błąd z ponowieniem widoczne przy samej sekcji. */
export function Card({
  title,
  actions,
  loading,
  error,
  onRetry,
  className,
  children,
}: {
  title?: ReactNode;
  actions?: ReactNode;
  loading?: boolean;
  error?: string | null;
  onRetry?: () => void;
  className?: string;
  children?: ReactNode;
}) {
  return (
    <section
      className={className ? `card ${className}` : "card"}
      aria-busy={loading || undefined}
      style={{
        background: token("elevation.surface.raised"),
        border: `${token("border.width")} solid ${token("color.border")}`,
        borderRadius: token("radius.large"),
        padding: `${token("space.150")} ${token("space.200")}`,
      }}
    >
      <Stack space="space.100">
        {(title || actions || loading) && (
          <Inline space="space.150" alignBlock="center" spread="space-between" shouldWrap>
            <Inline space="space.100" alignBlock="center">
              {title && (
                <Heading size="small" as="h2">
                  {title}
                </Heading>
              )}
              {loading && <Spinner size="small" label="loading…" />}
            </Inline>
            {actions && (
              <Inline space="space.100" alignBlock="center" shouldWrap>
                {actions}
              </Inline>
            )}
          </Inline>
        )}
        {error && <ErrorBox message={error} onRetry={onRetry} disabled={loading} />}
        {children}
      </Stack>
    </section>
  );
}

export function ErrorBox({ message, onRetry, disabled }: { message: string; onRetry?: () => void; disabled?: boolean }) {
  return (
    <div role="alert">
      <SectionMessage appearance="error">
        <Stack space="space.100">
          <Text as="p">{message}</Text>
          {onRetry && (
            <Inline>
              <Button onClick={onRetry} isDisabled={disabled}>
                Try again
              </Button>
            </Inline>
          )}
        </Stack>
      </SectionMessage>
    </div>
  );
}

export function Empty({ children }: { children: ReactNode }) {
  return (
    <div
      style={{
        padding: token("space.200"),
        border: `${token("border.width")} dashed ${token("color.border")}`,
        borderRadius: token("radius.medium"),
      }}
    >
      <Text as="p" color="color.text.subtlest">
        {children}
      </Text>
    </div>
  );
}

export function Loading({ children = "Loading…" }: { children?: ReactNode }) {
  return (
    <div role="status">
      <Inline space="space.100" alignBlock="center">
        <Spinner size="small" />
        <Text color="color.text.subtle">{children}</Text>
      </Inline>
    </div>
  );
}

/**
 * Obejście dla ModalDialog w StrictMode (dev, `npm run dev` w docker compose): Portal Atlaskit usuwa swój kontener
 * przy podwójnym wywołaniu efektów i dołącza go z powrotem dopiero przy kolejnym renderze — bez niego okno bez dalszych
 * renderów się nie pokazuje, a po ponownym dołączeniu traci fokus (`onSettled` ustawia go jeszcze raz).
 * W buildzie produkcyjnym niepotrzebne. Wywołaj w komponencie, który renderuje `ModalDialog`.
 * shortcut: czasowy dodatkowy render; gdy ADS włączy flagę platform_design_system_team_portal_logic_r18_fix, usunąć.
 */
export function useRerenderOnMount(onSettled?: () => void) {
  const [, rerender] = useReducer((n: number) => n + 1, 0);
  const settled = useRef(onSettled);
  settled.current = onSettled;
  useEffect(() => {
    const t = setTimeout(() => {
      flushSync(rerender);
      settled.current?.();
    }, 50);
    return () => clearTimeout(t);
  }, []);
}

/** Szkielet listy na czas pierwszego ładowania (zajmuje mniej więcej tyle miejsca co wiersze). */
export function ListSkeleton({ rows = 5, label = "Loading…" }: { rows?: number; label?: string }) {
  return (
    <div role="status" aria-label={label}>
      <Stack space="space.100">
        {Array.from({ length: rows }, (_, i) => (
          <Skeleton key={i} width="100%" height={token("space.500")} isShimmering />
        ))}
      </Stack>
    </div>
  );
}

/** Stan pusty listy: nagłówek + opis (ADS EmptyState, bez ilustracji). */
export function EmptyList({ header, description }: { header: string; description?: ReactNode }) {
  return <EmptyState header={header} description={description} headingLevel={2} />;
}

/** Przełącznik kilku wartości (np. filtr „mój zespół / wszystkie”). ADS nie ma segmented control → ButtonGroup. */
export function Segmented<T extends string>({
  options,
  value,
  onChange,
  label,
}: {
  options: { value: T; label: ReactNode }[];
  value: T;
  onChange: (v: T) => void;
  label: string;
}) {
  return (
    <div className="segmented">
      <ButtonGroup label={label}>
        {options.map((o) => (
          <Button key={o.value} isSelected={o.value === value} aria-pressed={o.value === value} onClick={() => onChange(o.value)}>
            {o.label}
          </Button>
        ))}
      </ButtonGroup>
    </div>
  );
}

/** Zakładki przełączające widok w obrębie sekcji; `children` (opcjonalnie) = treść panelu bieżącej zakładki. */
export function Tabs<T extends string>(props: {
  tabs: { value: T; label: ReactNode }[];
  value: T;
  onChange: (v: T) => void;
  /** Nazwa grupy zakładek — ADS TabList nie przyjmuje etykiety, więc nazywa ją otaczająca grupa. */
  label: string;
  children?: ReactNode;
}) {
  const { tabs, value, onChange, label, children } = props;
  const id = useId();
  return (
    <div role="group" aria-label={label}>
      <AdsTabs id={id} selected={Math.max(0, tabs.findIndex((t) => t.value === value))} onChange={(i) => onChange(tabs[i].value)}>
        <TabList>
          {tabs.map((t) => (
            <Tab key={t.value}>{t.label}</Tab>
          ))}
        </TabList>
        {children !== undefined && tabs.map((t) => <TabPanel key={t.value}>{t.value === value ? children : null}</TabPanel>)}
      </AdsTabs>
    </div>
  );
}

/** Pole formularza: widoczna etykieta, podpowiedź i błąd powiązane z kontrolką przez id/aria. */
export function Field({
  label,
  hint,
  error,
  children,
}: {
  label: ReactNode;
  hint?: ReactNode;
  error?: string | null;
  children: (props: { id: string; "aria-describedby"?: string; "aria-invalid"?: boolean }) => ReactNode;
}) {
  const id = useId();
  const hintId = hint ? `${id}-hint` : undefined;
  const errorId = error ? `${id}-error` : undefined;
  const describedBy = [hintId, errorId].filter(Boolean).join(" ") || undefined;
  return (
    <div className={error ? "field has-error" : "field"}>
      <Label htmlFor={id}>{label}</Label>
      {children({ id, "aria-describedby": describedBy, "aria-invalid": error ? true : undefined })}
      {hint && (
        <div id={hintId}>
          <HelperMessage>{hint}</HelperMessage>
        </div>
      )}
      {error && (
        <div id={errorId}>
          <ErrorMessage>{error}</ErrorMessage>
        </div>
      )}
    </div>
  );
}

/** Akcja wymagająca potwierdzenia w miejscu (bez okna dialogowego): „Usuń” → „Tak, usuń / Anuluj”. */
export function ConfirmInline({
  label,
  question,
  confirmLabel,
  onConfirm,
  busy,
  disabled,
}: {
  label: ReactNode;
  question: ReactNode;
  confirmLabel: ReactNode;
  onConfirm: () => void;
  busy?: boolean;
  disabled?: boolean;
}) {
  const [asking, setAsking] = useState(false);
  if (!asking)
    return (
      <Button appearance="danger" onClick={() => setAsking(true)} isDisabled={disabled || busy}>
        {label}
      </Button>
    );
  return (
    <span role="group" style={{ display: "inline-flex", alignItems: "center", gap: token("space.100"), flexWrap: "wrap" }}>
      <Text>{question}</Text>
      <Button
        appearance="danger"
        isDisabled={busy}
        onClick={() => {
          onConfirm();
          setAsking(false);
        }}
      >
        {confirmLabel}
      </Button>
      <Button appearance="subtle" onClick={() => setAsking(false)} isDisabled={busy} autoFocus>
        Cancel
      </Button>
    </span>
  );
}

const CopyIcon = adsIcon(Copy);

/** Kopiowanie tekstu do schowka z informacją o wyniku dla czytników ekranu. */
export function CopyButton({ text, label = "Copy" }: { text: string; label?: string }) {
  const [status, setStatus] = useState<"idle" | "ok" | "failed">("idle");
  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
      setStatus("ok");
    } catch {
      setStatus("failed");
    }
    setTimeout(() => setStatus("idle"), 2000);
  }
  return (
    <>
      <Tooltip content={text}>
        {(tooltipProps) => (
          <Button {...tooltipProps} spacing="compact" iconBefore={CopyIcon} onClick={copy} aria-label={`${label}: ${text}`}>
            {label}
          </Button>
        )}
      </Tooltip>
      <span role="status">
        {status !== "idle" && (
          <Text size="small" color={status === "ok" ? "color.text.success" : "color.text.danger"}>
            {status === "ok" ? "Copied" : "Failed to copy"}
          </Text>
        )}
      </span>
    </>
  );
}

/** Status domeny (status.ts) jako Lozenge z ikoną i etykietą — nigdy sam kolor. */
export function StatusLozenge<D extends StatusDomain>({ domain, value }: { domain: D; value: StatusValue<D> | string | boolean }) {
  const { label, appearance, isBold, icon: Icon } = statusOf(domain, value);
  return (
    <Lozenge appearance={appearance} isBold={isBold}>
      <span style={{ display: "inline-flex", alignItems: "center", gap: token("space.050") }}>
        <Icon size={12} aria-hidden />
        {label}
      </span>
    </Lozenge>
  );
}

/** Nagłówek ekranu: tytuł (h1), akcje po prawej, opcjonalny pasek filtrów pod spodem. */
export function PageHeader({ children, actions, bottomBar }: { children: ReactNode; actions?: ReactElement; bottomBar?: ReactElement }) {
  return (
    <AdsPageHeader actions={actions} bottomBar={bottomBar}>
      {children}
    </AdsPageHeader>
  );
}

type FieldAria = { id?: string; "aria-describedby"?: string; "aria-invalid"?: boolean };

/** Lista wyboru z API zbliżonym do `<select>`: wartość jako string, `onChange(value)`. */
export function Select({
  value,
  options,
  onChange,
  placeholder,
  isDisabled,
  "aria-label": ariaLabel,
  ...aria
}: FieldAria & {
  value: string;
  options: { value: string; label: string }[];
  onChange: (value: string) => void;
  placeholder?: string;
  isDisabled?: boolean;
  "aria-label"?: string;
}) {
  return (
    // eslint-disable-next-line @atlaskit/design-system/no-placeholder -- prymityw tylko przekazuje placeholder z ekranu (bez niego ADS pokazuje „Select...”)
    <AdsSelect<{ value: string; label: string }>
      inputId={aria.id}
      aria-describedby={aria["aria-describedby"]}
      aria-label={ariaLabel}
      isInvalid={aria["aria-invalid"]}
      isDisabled={isDisabled}
      placeholder={placeholder}
      options={options}
      value={options.find((o) => o.value === value) ?? null}
      onChange={(o) => onChange(o?.value ?? "")}
    />
  );
}

type TextfieldProps = Parameters<typeof Textfield>[0];
type TextAreaProps = Parameters<typeof AdsTextArea>[0];

/** Pole tekstowe (Textfield); przyjmuje propsy natywnego `<input>`, `aria-invalid` z `Field` zaznacza błąd. */
export const TextInput = forwardRef<HTMLInputElement, TextfieldProps & { disabled?: boolean }>(function TextInput({ disabled, ...props }, ref) {
  // eslint-disable-next-line @atlaskit/design-system/no-readonly-or-disabled-inputs -- prymityw przekazuje `disabled` z ekranu
  return <Textfield {...props} ref={ref} isDisabled={props.isDisabled ?? disabled} isInvalid={props.isInvalid ?? props["aria-invalid"] === true} />;
});

/** Pole wielowierszowe (TextArea); jak `TextInput`. */
export function TextArea({ disabled, ...props }: TextAreaProps & { disabled?: boolean }) {
  // eslint-disable-next-line @atlaskit/design-system/no-readonly-or-disabled-inputs -- prymityw przekazuje `disabled` z ekranu
  return <AdsTextArea {...props} isDisabled={props.isDisabled ?? disabled} isInvalid={props.isInvalid ?? props["aria-invalid"] === true} />;
}

/** Pole wyboru z etykietą (Checkbox ADS, API bez zmian). */
export { Checkbox } from "@atlaskit/checkbox";
