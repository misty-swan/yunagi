# Verify a download

Yunagi does not use Apple notarization or a Windows code-signing certificate (paid developer registration). Because of that, your OS may warn that it cannot verify the developer. Instead, you can check that a download is the official one by following these steps.

When you install with the install script, the script compares the SHA-256 hash for you (and installs nothing if it does not match). These steps are for files you downloaded by hand, and for when you want to check on your own, separately from the script.

There are two things to check.

1. `SHA256SUMS`, the list of hashes of every download, is signed with Yunagi's release key.
2. The hash of the file you downloaded is the same as the one in that list.

## 1. The public key for downloads

<!-- slot:key -->

The release key is separate from the key that signs in-app updates. The public key is published on this site and in the public repository. If you compare the two copies, you will notice if one of them was altered. If the key is ever replaced, it will be announced in the [news](/en/news/).

## 2. Collect what you need

Save these four things in the same folder.

- The download you want to check (get it from "Download manually" on the [Download](/en/download/) page)
- `SHA256SUMS`
- `SHA256SUMS.sig`
- `yunagi-release.pub` (the public key above)

To check the signature, use [minisign](https://jedisct1.github.io/minisign/). How to install it is on its official page.

## 3. Check the signature of SHA256SUMS

```sh
minisign -Vm SHA256SUMS -x SHA256SUMS.sig -p yunagi-release.pub
```

If it prints `Signature and comment signature verified`, `SHA256SUMS` was signed with the release key. If it fails, do not use that file.

## 4. Compare the hash

Compare the hash of the downloaded file with the signed `SHA256SUMS`. Replace `1.2.3` and `arm64` with the name of your file.

Linux:

```sh
sha256sum -c --ignore-missing SHA256SUMS
```

macOS:

```sh
grep ' Yunagi_1.2.3_arm64.app.tar.gz$' SHA256SUMS | shasum -a 256 -c -
```

Windows (PowerShell):

```powershell
(Get-FileHash .\Yunagi_1.2.3_x86_64-setup.exe -Algorithm SHA256).Hash.ToLower()
Select-String -Path .\SHA256SUMS -Pattern 'Yunagi_1.2.3_x86_64-setup.exe'
```

On Linux and macOS, `OK` means they match. On Windows, they match if the hash printed on the first line (64 digits) is the same as the first 64 digits of the line printed on the second.

## 5. If you could not verify it

Do not use that file. If downloading it again gives the same result, tell us through the [Security](/en/security/) contact.
