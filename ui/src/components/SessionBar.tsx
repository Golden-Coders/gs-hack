import Button from "@atlaskit/button/new";
import { Stack, Text } from "@atlaskit/primitives/compiled";
import { logout, usePersonId, usePersonName } from "../api/session";

export default function SessionBar() {
  const personId = usePersonId();
  const personName = usePersonName();

  return (
    <Stack space="space.100">
      <div>
        <Text size="small" color="color.text.subtle">
          Signed in as:
        </Text>
        <div>
          <Text weight="semibold">{personName ?? personId ?? "—"}</Text>
        </div>
      </div>
      {personId && (
        <Button appearance="subtle" onClick={() => void logout()}>
          Sign out
        </Button>
      )}
    </Stack>
  );
}
