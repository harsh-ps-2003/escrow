# Escrow Module

So in happy state :

Buyer burns the ecash (using mint module) and the [uuid of escrow : buyer + seller + arbiter + amount + hash256(code)] is inserted to federation DB representing the escrow formation, seller sends the product, if the product is cool and buyer wants to use it, buyer sends the seller a code in private before or at the refund day though the website/app they bought the product from, buyer claims the ecash from federation by giving the code to federation (through command line seller enters the code and the escrow module checks whether code matches the DB code and then federation pubkey gives ecash to seller pubkey). So buyer -> federation -> seller.

Input will be ecash(mint module) and output will be a data structure that encapsulate buyer + seller + arbiter + amount. This data will be shared in guardians' DB as consented upon state

buyer will create an escrow with `create escrow <buyer-pubkey> <seller-pubkey> <arbiter-pubkey> <cost of product>`
Returns `<escrow-id> <secret-code>` to seller. The secret-code is randomly generated and given to buyer to give to seller if happy with product and hash256(secret-code) is stored in DB.

Get info (pubkey of all 3 and cost of product) about folks involved in escrow - `escrow info <escrow-id>`

Seller Claim ecash from secret code obtained by happy buyer - `escrow claim <escrow-id> <secret-code>`

Either of them call arbiter - `escrow dispute <escrow-id>`

// create mermaid diagram to explain the architecture of the escrow!