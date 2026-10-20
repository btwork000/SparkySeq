# Éditeur de patterns (fork)

Onglet **Patterns** → choisir un emplacement → **Éditer…** (ou **Créer un pattern ici…**
sur un emplacement vide).

## Utilisation

- **Grille** : 1/4, 1/8, 1/16 ou 1/32 (1/16 par défaut, le choix est retenu). Les notes
  se calent dessus quand on les pose, les déplace ou les allonge. 16 pads affichés ; les
  boutons A à J changent de banque (le chiffre indique les notes de chaque banque).
- **Clic** sur une case vide : ajoute une note (même longueur et vélocité que la
  dernière). Glisser tout de suite après le clic l'allonge.
- **Glisser une note** : la déplacer dans le temps ou sur un autre pad.
- **Bord droit d'une note** : régler sa longueur.
- **Clic droit** ou **double-clic** : supprimer. Touche Suppr aussi.
- Note sélectionnée : vélocité et longueur (en pas de grille) dans la barre ; flèches
  haut/bas = vélocité, gauche/droite = déplacer d'un pas de grille. **Ctrl+Z** annule.
- **Mesures** : 2, 4, 8 ou 16. Raccourcir demande confirmation si des notes disparaissent.
- **Écouter sur la 404** : chaque note posée ou déplacée sonne sur la machine. Maintenir
  le nom d'un pad à gauche le fait sonner.
- Rien ne part vers la machine avant **Envoyer vers la 404**. La lecture du pattern se
  fait ensuite sur la 404.

## Règles

- Deux notes sur un même pad s'excluent : une note s'arrête au début de la suivante.
- Les notes hors grille (enregistrées en live) gardent leur position exacte ; elles sont
  en pointillés. Une note déplacée se cale sur la grille.
- Les octets non compris (contrôles, enregistrements inconnus, pitch, drapeaux) sont
  recopiés tels quels.
- 4/4 uniquement : les autres signatures s'ouvrent en lecture seule.

## Sécurité

- Avant de remplacer un pattern, l'ancien fichier est copié sur l'ordinateur, dans le
  dossier de données de l'app, sous `pattern-backups/PROJECT_NN/`. Le message après
  l'envoi donne le chemin exact.
- L'envoi n'agit que sur le projet sélectionné sur la 404 ; si un autre projet a été
  choisi entre-temps, rien n'est écrit.

## Hypothèses à vérifier sur la machine

- **Envoi léger** : seul le fichier du pattern est écrit, puis la 404 recharge le projet
  (commande `B1` de la restauration, sans l'effacement préalable). Jamais essayé avant :
  premier test sur un projet de brouillon, avec une sauvegarde.
- **Pitch d'une note neuve** : `0x8D`, la valeur que la 404 écrit pour une note non
  transposée.
- **Longueur** : stockée en ticks moins un (une double croche = 119).

Pour analyser un fichier de pattern sans la machine :

```
cargo run -p sp404-formats --example ptn_dump -- PTN00001.BIN
cargo run -p sp404-formats --example ptn_roundtrip -- PTN*.BIN
```
