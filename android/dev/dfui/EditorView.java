package dev.dfui;

import android.content.Context;
import android.text.InputType;
import android.view.View;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;

/** The view the soft keyboard attaches to; NativeActivity has none of its own. */
public final class EditorView extends View {
    private static native void text(String s);

    /** Deletes this many UTF-16 units before the caret. */
    private static native void backspace(int units);

    private int inputType = InputType.TYPE_CLASS_TEXT;

    public EditorView(Context c) {
        super(c);
        setFocusable(true);
        setFocusableInTouchMode(true);
    }

    /** Takes effect when the input connection restarts. */
    public void setInputType(int type) {
        inputType = type;
    }

    @Override
    public boolean onCheckIsTextEditor() {
        return true;
    }

    @Override
    public InputConnection onCreateInputConnection(EditorInfo out) {
        out.inputType = inputType;
        out.imeOptions = EditorInfo.IME_FLAG_NO_FULLSCREEN | EditorInfo.IME_FLAG_NO_EXTRACT_UI;
        if ((inputType & InputType.TYPE_MASK_VARIATION) == InputType.TYPE_TEXT_VARIATION_PASSWORD) {
            out.imeOptions |= EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING;
        }
        return new Connection(this);
    }

    /** Turns the keyboard's edits into typed text and backspaces. */
    private static final class Connection extends BaseInputConnection {
        private String composing = "";

        Connection(View v) {
            super(v, false);
        }

        private void replace(String next) {
            int same = 0;
            while (same < composing.length()
                    && same < next.length()
                    && composing.charAt(same) == next.charAt(same)) {
                same++;
            }
            int removed = composing.length() - same;
            if (removed > 0) {
                backspace(removed);
            }
            if (same < next.length()) {
                text(next.substring(same));
            }
            composing = next;
        }

        @Override
        public boolean setComposingText(CharSequence t, int cursor) {
            replace(t.toString());
            return true;
        }

        @Override
        public boolean commitText(CharSequence t, int cursor) {
            replace(t.toString());
            composing = "";
            return true;
        }

        @Override
        public boolean finishComposingText() {
            composing = "";
            return true;
        }

        @Override
        public boolean deleteSurroundingText(int before, int after) {
            composing = "";
            if (before > 0) {
                backspace(before);
            }
            return true;
        }
    }
}
