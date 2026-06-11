# RATO shell integration. Source from ~/.bashrc:
#   source ~/rato/packaging/shell/rat-init.sh
alias rat="$HOME/rato/target/release/rat"
eval "$("$HOME/rato/target/release/rat" shell-init bash)"
