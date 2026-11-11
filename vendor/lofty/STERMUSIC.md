# lofty 0.22.4, corrigé pour SterMusic

Copie de lofty 0.22.4 (crates.io), avec un seul correctif, repris de lofty 0.25 :
`src/id3/v2/read.rs` et `src/id3/v2/frame/read.rs`.

Dans un tag ID3v2.4 dont l'en-tête porte le drapeau de désynchronisation, lofty 0.22 la défaisait
une fois sur tout le tag, puis une seconde fois dans chaque cadre marqué. La seconde passe
retirait des octets `00` qui suivent un `FF` et font partie des données : une pochette JPEG
perdait ainsi quelques milliers d'octets et se décodait en un carré gris.

Lofty 0.25 corrige cela, mais son API a changé (dates, clés) : la copie évite de réécrire la
lecture et l'écriture des tags. À retirer en passant à lofty 0.25 ou plus.
