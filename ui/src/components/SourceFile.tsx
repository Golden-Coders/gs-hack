import { Code } from "@atlaskit/code";
import Lozenge from "@atlaskit/lozenge";
import { Box, Inline, Text } from "@atlaskit/primitives/compiled";
import { CopyButton } from "../ui/primitives";

export function ReadOnlyNote({ children }: { children: React.ReactNode }) {
  return (
    <Box paddingBlockEnd="space.200">
      <Inline space="space.100" alignBlock="center" shouldWrap>
        <Lozenge>Tylko do odczytu</Lozenge>
        <Text color="color.text.subtle">{children}</Text>
      </Inline>
    </Box>
  );
}

export default function SourceFile({ path, label = "Edytuj w:" }: { path?: string | null; label?: string }) {
  return (
    <Box paddingBlock="space.100">
      <Inline space="space.100" alignBlock="center" shouldWrap>
        <Text size="small" color="color.text.subtle">
          {label}
        </Text>
        {path ? (
          <>
            <Code>{path}</Code>
            <CopyButton text={path} />
          </>
        ) : (
          <Text color="color.text.subtle">brak ścieżki pliku w odpowiedzi API</Text>
        )}
      </Inline>
    </Box>
  );
}
