// Markdown runbooków MANUAL. react-markdown nie renderuje surowego HTML i czyści niebezpieczne URL-e.
import ReactMarkdown from "react-markdown";

export default function Markdown({ source }: { source: string }) {
  return (
    <div className="markdown">
      <ReactMarkdown
        components={{ a: ({ node: _node, ...props }) => <a {...props} target="_blank" rel="noreferrer noopener" /> }}
      >
        {source}
      </ReactMarkdown>
    </div>
  );
}
