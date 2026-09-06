/** Single-line composer used for feedback. Owns the keyboard while active. */
import { Box, Text, useInput } from "ink";
import { useState } from "react";

import { theme } from "./theme.js";

export interface TextInputProps {
  placeholder: string;
  width: number;
  onSubmit: (value: string) => void;
  onCancel: () => void;
  submitLabel?: string;
}

export function TextInput({ placeholder, width, onSubmit, onCancel, submitLabel }: TextInputProps) {
  const [value, setValue] = useState("");
  const [cursor, setCursor] = useState(0);

  useInput((input, key) => {
    if (key.escape) {
      onCancel();
      return;
    }
    // A paste (or a fast PTY) delivers several characters at once, possibly
    // ending in a newline. Insert the printable part, then submit if asked.
    const pasted = input.replace(/[\r\n]/g, "");
    const wantsSubmit = key.return || /[\r\n]/.test(input);
    if (pasted.length > 1 && !key.ctrl && !key.meta) {
      const next = value.slice(0, cursor) + pasted + value.slice(cursor);
      setValue(next);
      setCursor(cursor + pasted.length);
      if (wantsSubmit) onSubmit(next);
      return;
    }
    if (wantsSubmit) {
      onSubmit(value);
      return;
    }
    if (key.backspace || key.delete) {
      if (cursor > 0) {
        setValue(value.slice(0, cursor - 1) + value.slice(cursor));
        setCursor(cursor - 1);
      }
      return;
    }
    if (key.leftArrow) {
      setCursor(Math.max(0, cursor - 1));
      return;
    }
    if (key.rightArrow) {
      setCursor(Math.min(value.length, cursor + 1));
      return;
    }
    if (key.ctrl && input === "a") {
      setCursor(0);
      return;
    }
    if (key.ctrl && input === "e") {
      setCursor(value.length);
      return;
    }
    if (key.ctrl && input === "u") {
      setValue("");
      setCursor(0);
      return;
    }
    if (key.ctrl || key.meta || key.tab || key.upArrow || key.downArrow) return;
    if (!input) return;
    setValue(value.slice(0, cursor) + input + value.slice(cursor));
    setCursor(cursor + input.length);
  });

  const innerWidth = Math.max(4, width - 2);
  // Keep the caret visible by scrolling the text horizontally.
  const start = Math.max(0, cursor - innerWidth + 1);
  const visible = value.slice(start, start + innerWidth);
  const caretIndex = cursor - start;
  const before = visible.slice(0, caretIndex);
  const at = visible.charAt(caretIndex) || " ";
  const after = visible.slice(caretIndex + 1);

  return (
    <Box flexDirection="column" width={width}>
      <Box borderStyle="round" borderColor={theme.blue} paddingX={1} width={width}>
        {value.length === 0 ? (
          <Text>
            <Text inverse> </Text>
            <Text color={theme.muted}>{placeholder.slice(0, innerWidth - 1)}</Text>
          </Text>
        ) : (
          <Text>
            {before}
            <Text inverse>{at}</Text>
            {after}
          </Text>
        )}
      </Box>
      <Text color={theme.muted}>
        {"  ⏎ "}
        {submitLabel ?? "send"}
        {"  ·  esc cancel"}
      </Text>
    </Box>
  );
}
