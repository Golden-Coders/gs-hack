// Markdown runbooków MANUAL. react-markdown nie renderuje surowego HTML i czyści niebezpieczne URL-e.
import Link from "@atlaskit/link";
import { token } from "@atlaskit/tokens";
import ReactMarkdown from "react-markdown";

export default function Markdown({ source }: { source: string }) {
  return (
    <div
      style={{
        background: token("elevation.surface.raised"),
        border: `${token("border.width")} solid ${token("color.border")}`,
        borderRadius: token("radius.large"),
        padding: `${token("space.050")} ${token("space.200")} ${token("space.150")}`,
        marginBottom: token("space.150"),
      }}
    >
      <ReactMarkdown
        components={{
          a: ({ href, children }) => (
            <Link href={href ?? ""} target="_blank" rel="noreferrer noopener" newWindowLabel="(otwiera nowe okno)">
              {children}
            </Link>
          ),
        }}
      >
        {source}
      </ReactMarkdown>
    </div>
  );
}
