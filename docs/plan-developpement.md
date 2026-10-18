# Plan de développement (fork)

## Fait

- **Éditeur de patterns** : voir [editeur-patterns.md](editeur-patterns.md). À tester sur la
  404 (envoi léger, pitch, longueurs).

## À faire

### Fondu de boucle (onglet Samples)

Pour les nappes et les drones : supprimer le clic au point de bouclage en gravant un
fondu dans le sample, sans toucher au firmware.

- **Bouton** « Fondu de boucle… » à côté de Normalize.
- **Réglage** : longueur du fondu en millisecondes, 500 ms par défaut.
- **Forme** : une seule, à puissance constante (pas de sélecteur).
- **Principe** : les N derniers échantillons avant E (fin) sont mélangés avec les N
  échantillons qui précèdent L (loop top). Au saut de E vers L, la lecture retombe sur la
  suite naturelle du son.
- **Boucle depuis le début (L = S)** : rien avant L. L'app avance alors L de la durée du
  fondu et mélange la fin avec ce début. Refus avec un message si la boucle est plus
  courte que le fondu.
- **Pas de recalage** sur les passages à zéro (inutile avec des fondus longs).
- **Sécurité** : l'original est gardé sur l'ordinateur avant de remplacer le sample.
- **Matière existante dans SparkyMK2** : lecture du sample d'un pad, points S/E/L, import
  d'un sample sur un pad avec ses réglages.

### Pitch par note dans l'éditeur

À reprendre après les tests sur la 404 : l'octet de pitch existe sous deux formes (avec et
sans le bit 7), probablement le mode chromatique et le pitch note par note.
