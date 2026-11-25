#!/bin/sh
# Fake agent for tests. Reports state with OSC 7501, as Pi does, so no
# model is needed. Each input line is one turn:
#   block   -> asks a question (blocked), the next line answers it
#   fail    -> error
#   exit N  -> exits with code N
#   slow    -> works for 3 s
#   other   -> prints "reply: <line>" and is done
osc() { printf '\033]7501;state=%s:app=fake\033\\' "$1"; }
osc idle
while IFS= read -r line; do
  osc working
  case "$line" in
    block)
      echo "approve? 1 yes 2 no"
      osc blocked
      IFS= read -r answer
      osc working
      echo "answer: $answer"
      ;;
    fail) echo "failed"; osc error; continue ;;
    exit\ *) exit "${line#exit }" ;;
    slow) sleep 3 ;;
    *) echo "reply: $line" ;;
  esac
  sleep 0.3
  osc done
done
